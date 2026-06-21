use anyhow::Result;
use redis::{aio::ConnectionManager, AsyncCommands};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, string::{String, ToString}, sync::Arc, time::{SystemTime, UNIX_EPOCH}};
use tokio::sync::RwLock;

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

    pub async fn get_raw(&self, key: &CacheKey) -> Result<Option<String>> {
        match self {
            Self::Redis(c) => c.get_raw(&key.as_string()).await,
            Self::InMemory(c) => c.get_raw(key).await,
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
            Some(v) => Ok(Some(serde_json::from_str(&v)?)),
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
}

impl RedisCache {
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let connection = ConnectionManager::new(client).await?;
        Ok(Self { connection })
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
}

pub struct InMemoryCache {
    store: Arc<RwLock<HashMap<CacheKey, Entry>>>,
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
        }
    }

    async fn check_connection(&self) -> bool {
        true
    }

    async fn get_raw(&self, key: &CacheKey) -> Result<Option<String>> {
        let store = self.store.read().await;
        match store.get(key) {
            Some(entry) if entry.expires_at > now_secs() => Ok(Some(entry.value.clone())),
            _ => Ok(None),
        }
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
        assert_eq!(cache.get_raw(&key).await.unwrap(), Some("value".to_string()));
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
        cache.set_raw(&v_key, "verification_val", 60).await.unwrap();
        cache.set_raw(&c_key, "config_val", 60).await.unwrap();
        assert_eq!(cache.get_raw(&v_key).await.unwrap(), Some("verification_val".to_string()));
        assert_eq!(cache.get_raw(&c_key).await.unwrap(), Some("config_val".to_string()));
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let cache = CacheBackend::InMemory(InMemoryCache::new());
        let key = CacheKey::Verification("del".to_string());
        cache.set_raw(&key, "v", 60).await.unwrap();
        cache.delete(&key).await.unwrap();
        assert_eq!(cache.get_raw(&key).await.unwrap(), None);
    }
}
