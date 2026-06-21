use anyhow::Result;
use redis::{aio::ConnectionManager, AsyncCommands};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, string::{String, ToString}, sync::Arc, time::{SystemTime, UNIX_EPOCH}};
use std::collections::HashMap;
use std::prelude::v1::*;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::metrics::MetricsRegistry;

/// Typed cache key variants to prevent key collisions across namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CacheKey {
    Verification(String),
    Config(String),
}

impl CacheKey {
    pub fn as_string(&self) -> String {
        match self {
            CacheKey::Verification(hash) => format!("verification:{}", hash),
            CacheKey::Config(key) => format!("config:{}", key),
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

struct Entry {
    value: String,
    expires_at: u64,
}

pub enum CacheBackend {
    Redis(RedisCache),
    InMemory(InMemoryCache),
}

impl CacheBackend {
    pub async fn check_connection(&self) -> bool {
        match self {
            Self::Redis(c) => c.check_connection().await,
            Self::InMemory(c) => c.check_connection().await,
        }
    }

    /// Retrieve a raw cached value. Emits cache hit/miss/expired metrics.
    pub async fn get_raw(&self, key: &CacheKey) -> Result<Option<String>> {
        match self {
            Self::Redis(c) => {
                // Redis handles TTL natively — misses include expired entries
                let result = c.get_raw(&key.as_string()).await?;
                match &result {
                    Some(_) => c.record_hit(),
                    None => c.record_miss(),
                }
                Ok(result)
            }
            Self::InMemory(c) => {
                // InMemory distinguishes expired from true miss
                let (value, was_expired) = c.get_raw_with_expiry(key).await?;
                if was_expired {
                    c.record_expired();
                } else if value.is_some() {
                    c.record_hit();
                } else {
                    c.record_miss();
                }
                Ok(value)
            }
        }
    }

    pub async fn set_raw(&self, key: &CacheKey, value: &str, ttl: u64) -> Result<()> {
        match self {
            Self::Redis(c) => c.set_raw(&key.as_string(), value, ttl).await,
            Self::InMemory(c) => c.set_raw(key, value, ttl).await,
        }
    }

    pub async fn get<T>(&self, key: &CacheKey) -> Result<Option<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        match self.get_raw(key).await? {
            Some(v) => match serde_json::from_str(&v) {
                Ok(parsed) => Ok(Some(parsed)),
                Err(_) => {
                    // Record serialization failure on any backend
                    match self {
                        Self::Redis(c) => c.record_serialization_failure(),
                        Self::InMemory(c) => c.record_serialization_failure(),
                    }
                    Ok(None)
                }
            },
            None => Ok(None),
        }
    }

    pub async fn set<T>(&self, key: &CacheKey, value: &T, ttl: u64) -> Result<()>
    where
        T: Serialize,
    {
        let serialized = serde_json::to_string(value)?;
        self.set_raw(key, &serialized, ttl).await
    }

    pub async fn delete(&self, key: &CacheKey) -> Result<()> {
        match self {
            Self::Redis(c) => c.delete(&key.as_string()).await,
            Self::InMemory(c) => c.delete(key).await,
        }
    }
}

pub struct RedisCache {
    connection: ConnectionManager,
    metrics: Option<Arc<MetricsRegistry>>,
}

impl RedisCache {
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let connection = ConnectionManager::new(client).await?;
        Ok(Self {
            connection,
            metrics: None,
        })
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsRegistry>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    async fn check_connection(&self) -> bool {
        let mut conn = self.connection.clone();
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .is_ok()
    }

    async fn get_raw(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.connection.clone();
        let value: Option<String> = conn.get(key).await?;
        Ok(value)
    }

    async fn set_raw(&self, key: &str, value: &str, ttl: u64) -> Result<()> {
        let mut conn = self.connection.clone();
        conn.set_ex::<_, _, ()>(key, value, ttl).await?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let mut conn = self.connection.clone();
        conn.del::<_, ()>(key).await?;
        Ok(())
    }

    fn record_hit(&self) {
        if let Some(ref m) = self.metrics {
            m.increment_cache_hits();
        }
    }

    fn record_miss(&self) {
        if let Some(ref m) = self.metrics {
            m.increment_cache_misses();
        }
    }

    #[allow(dead_code)]
    fn record_expired(&self) {
        if let Some(ref m) = self.metrics {
            m.increment_cache_expired();
        }
    }

    fn record_serialization_failure(&self) {
        if let Some(ref m) = self.metrics {
            m.increment_cache_serialization_failures();
        }
    }
}

pub struct InMemoryCache {
    store: Arc<RwLock<HashMap<CacheKey, Entry>>>,
    metrics: Option<Arc<MetricsRegistry>>,
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryCache {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            metrics: None,
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsRegistry>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    async fn check_connection(&self) -> bool {
        true
    }

    /// Returns (value, was_expired).
    async fn get_raw_with_expiry(&self, key: &CacheKey) -> Result<(Option<String>, bool)> {
        let store = self.store.read().await;
        match store.get(key) {
            Some(entry) if entry.expires_at > now_secs() => {
                Ok((Some(entry.value.clone()), false))
            }
            Some(_) => {
                // Entry exists but TTL has elapsed
                Ok((None, true))
            }
            None => Ok((None, false)),
        }
    }

    #[allow(dead_code)]
    async fn get_raw(&self, key: &CacheKey) -> Result<Option<String>> {
        let (value, _was_expired) = self.get_raw_with_expiry(key).await?;
        Ok(value)
    }

    async fn set_raw(&self, key: &CacheKey, value: &str, ttl: u64) -> Result<()> {
        let mut store = self.store.write().await;
        store.insert(
            key.clone(),
            Entry {
                value: value.to_string(),
                expires_at: now_secs().saturating_add(ttl),
            },
        );
        Ok(())
    }

    async fn delete(&self, key: &CacheKey) -> Result<()> {
        let mut store = self.store.write().await;
        store.remove(key);
        Ok(())
    }

    fn record_hit(&self) {
        if let Some(ref m) = self.metrics {
            m.increment_cache_hits();
        }
    }

    fn record_miss(&self) {
        if let Some(ref m) = self.metrics {
            m.increment_cache_misses();
        }
    }

    fn record_expired(&self) {
        if let Some(ref m) = self.metrics {
            m.increment_cache_expired();
        }
    }

    fn record_serialization_failure(&self) {
        if let Some(ref m) = self.metrics {
            m.increment_cache_serialization_failures();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn in_memory_cache_returns_value_within_ttl() {
        let cache = CacheBackend::InMemory(InMemoryCache::new());
        let key = CacheKey::Verification("abc".to_string());
        cache.set_raw(&key, "value", 60).await.unwrap();
        assert_eq!(
            cache.get_raw(&key).await.unwrap(),
            Some("value".to_string())
        );
    }

    #[tokio::test]
    async fn in_memory_cache_expires_entry_after_ttl() {
        let cache = CacheBackend::InMemory(InMemoryCache::new());
        let key = CacheKey::Verification("ttl_test".to_string());
        cache.set_raw(&key, "stale", 1).await.unwrap();
        sleep(Duration::from_secs(2)).await;
        assert_eq!(cache.get_raw(&key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn cache_keys_are_namespaced() {
        let v_key = CacheKey::Verification("x".to_string());
        let c_key = CacheKey::Config("x".to_string());
        assert_ne!(v_key.as_string(), c_key.as_string());
    }

    #[tokio::test]
    async fn different_namespaces_do_not_collide() {
        let cache = CacheBackend::InMemory(InMemoryCache::new());
        let v_key = CacheKey::Verification("same".to_string());
        let c_key = CacheKey::Config("same".to_string());
        cache
            .set_raw(&v_key, "verification_val", 60)
            .await
            .unwrap();
        cache.set_raw(&c_key, "config_val", 60).await.unwrap();
        assert_eq!(
            cache.get_raw(&v_key).await.unwrap(),
            Some("verification_val".to_string())
        );
        assert_eq!(
            cache.get_raw(&c_key).await.unwrap(),
            Some("config_val".to_string())
        );
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let cache = CacheBackend::InMemory(InMemoryCache::new());
        let key = CacheKey::Verification("del".to_string());
        cache.set_raw(&key, "v", 60).await.unwrap();
        cache.delete(&key).await.unwrap();
        assert_eq!(cache.get_raw(&key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn in_memory_cache_emits_hit_metric() {
        let metrics = MetricsRegistry::arc();
        let cache = InMemoryCache::new().with_metrics(Arc::clone(&metrics));
        let backend = CacheBackend::InMemory(cache);
        let key = CacheKey::Verification("metric_hit".to_string());

        backend.set_raw(&key, "value", 60).await.unwrap();
        backend.get_raw(&key).await.unwrap();

        let output = metrics.render();
        assert!(output.contains("cache_hits_total"));
    }

    #[tokio::test]
    async fn in_memory_cache_emits_miss_metric() {
        let metrics = MetricsRegistry::arc();
        let cache = InMemoryCache::new().with_metrics(Arc::clone(&metrics));
        let backend = CacheBackend::InMemory(cache);
        let key = CacheKey::Verification("metric_miss".to_string());

        backend.get_raw(&key).await.unwrap();

        let output = metrics.render();
        assert!(output.contains("cache_misses_total"));
    }

    #[tokio::test]
    async fn in_memory_cache_emits_expired_metric() {
        let metrics = MetricsRegistry::arc();
        let cache = InMemoryCache::new().with_metrics(Arc::clone(&metrics));
        let backend = CacheBackend::InMemory(cache);
        let key = CacheKey::Verification("metric_expired".to_string());

        backend.set_raw(&key, "stale", 1).await.unwrap();
        sleep(Duration::from_secs(2)).await;
        backend.get_raw(&key).await.unwrap();

        let output = metrics.render();
        assert!(output.contains("cache_expired_total"));
    }

    #[tokio::test]
    async fn in_memory_cache_emits_serialization_failure_metric() {
        let metrics = MetricsRegistry::arc();
        let cache = InMemoryCache::new().with_metrics(Arc::clone(&metrics));
        let backend = CacheBackend::InMemory(cache);
        let key = CacheKey::Verification("serial_fail".to_string());

        // Store invalid JSON
        backend.set_raw(&key, "not-valid-json", 60).await.unwrap();
        // Try to deserialize as a struct — should fail
        let result: Option<serde_json::Value> = backend.get(&key).await.unwrap();
        // serde_json::from_str on "not-valid-json" fails → serialization failure metric
        assert!(result.is_none());

        let output = metrics.render();
        assert!(output.contains("cache_serialization_failures_total"));
    }
}
