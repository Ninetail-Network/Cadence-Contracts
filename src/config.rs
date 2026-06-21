use std::prelude::v1::*;
use std::{env, fmt};
use std::sync::Arc;

use thiserror::Error;
use stellar_strkey::ed25519::PrivateKey;
use url::Url;

use crate::metrics::MetricsRegistry;

#[derive(Clone)]
pub struct AppConfig {
    pub port: u16,
    pub stellar_horizon_url: String,
    pub stellar_secret_key: Option<String>,
    pub redis_url: String,
    pub rate_limit_per_second: u32,
    pub rate_limit_burst: u32,
    /// Per-issuer rate limit: operations per second
    pub rate_limit_per_issuer_per_second: u32,
    /// Per-issuer rate limit: burst allowance
    pub rate_limit_per_issuer_burst: u32,
    /// Per-address rate limit: operations per second
    pub rate_limit_per_address_per_second: u32,
    /// Per-address rate limit: burst allowance
    pub rate_limit_per_address_burst: u32,
    pub stellar_max_retries: u32,
    pub log_level: String,
    pub webhook_urls: Vec<String>,
    pub webhook_secret: Option<String>,
    pub cache_verification_ttl: u64,
}

impl fmt::Debug for AppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppConfig")
            .field("port", &self.port)
            .field("stellar_horizon_url", &self.stellar_horizon_url)
            .field(
                "stellar_secret_key",
                &self.stellar_secret_key.as_deref().map(|_| "<redacted>"),
            )
            .field("redis_url", &self.redis_url)
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("rate_limit_burst", &self.rate_limit_burst)
            .field("stellar_max_retries", &self.stellar_max_retries)
            .field("log_level", &self.log_level)
            .field("webhook_urls", &self.webhook_urls)
            .field(
                "webhook_secret",
                &self.webhook_secret.as_deref().map(|_| "<redacted>"),
            )
            .field("cache_verification_ttl", &self.cache_verification_ttl)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration validation failed:\n{0}")]
    Validation(String),
}

impl AppConfig {
    /// Load configuration from environment variables.
    ///
    /// Records validation failures via the provided metrics registry.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_with_metrics(None)
    }

    /// Load configuration from environment variables, recording metrics if a registry is provided.
    pub fn from_env_with_metrics(
        metrics: Option<Arc<MetricsRegistry>>,
    ) -> Result<Self, ConfigError> {
        let mut errors = Vec::new();

        // Helper to read env var with default
        fn get_env_or_default(key: &str, default: &str) -> String {
            env::var(key).unwrap_or_else(|_| default.to_string())
        }

        // Basic string values with defaults
        let port_raw = get_env_or_default("PORT", "8080");
        let stellar_horizon_url =
            get_env_or_default("STELLAR_HORIZON_URL", "https://horizon-testnet.stellar.org");
        let redis_url = get_env_or_default("REDIS_URL", "redis://127.0.0.1:6379");
        let log_level = get_env_or_default("LOG_LEVEL", "info");
        let webhook_urls_raw = get_env_or_default("WEBHOOK_URLS", "");

        let stellar_secret_key = match env::var("STELLAR_SECRET_KEY") {
            Ok(key) => {
                if PrivateKey::from_string(&key).is_err() {
                    errors.push(
                        "STELLAR_SECRET_KEY must be a valid Stellar ed25519 secret key"
                            .to_string(),
                    );
                }
                Some(key)
            }
            Err(_) => {
                errors.push(
                    "STELLAR_SECRET_KEY is required but not set. Please set the environment variable."
                        .to_string(),
                );
                None
            }
        };
        let webhook_secret = env::var("WEBHOOK_SECRET").ok();

        // Numeric values with defaults
        let rate_limit_per_second_raw = get_env_or_default("RATE_LIMIT_PER_SECOND", "10");
        let rate_limit_burst_raw =
            get_env_or_default("RATE_LIMIT_BURST", &rate_limit_per_second_raw);
        let stellar_max_retries_raw = get_env_or_default("STELLAR_MAX_RETRIES", "3");
        let cache_verification_ttl_raw = get_env_or_default("CACHE_VERIFICATION_TTL", "3600");

        // Parse and validate port
        let port: u16 = match port_raw.parse() {
            Ok(p) if p > 0 => p,
            Ok(_) => {
                errors.push("PORT must be between 1 and 65535".to_string());
                8080
            }
            Err(_) => {
                errors.push(format!("PORT must be a valid u16, got '{}'", port_raw));
                8080
            }
        };

        // Validate horizon URL
        if Url::parse(&stellar_horizon_url).is_err() {
            errors.push(format!(
                "STELLAR_HORIZON_URL must be a valid URL, got '{}'",
                stellar_horizon_url
            ));
        }

        // Parse numeric values
        let rate_limit_per_second: u32 = match rate_limit_per_second_raw.parse() {
            Ok(v) if v > 0 => v,
            Ok(_) => {
                errors.push("RATE_LIMIT_PER_SECOND must be greater than 0".to_string());
                10
            }
            Err(_) => {
                errors.push(format!(
                    "RATE_LIMIT_PER_SECOND must be a valid u32, got '{}'",
                    rate_limit_per_second_raw
                ));
                10
            }
        };

        let rate_limit_burst: u32 = match rate_limit_burst_raw.parse() {
            Ok(v) => v,
            Err(_) => {
                errors.push(format!(
                    "RATE_LIMIT_BURST must be a valid u32, got '{}'",
                    rate_limit_burst_raw
                ));
                rate_limit_per_second
            }
        };

        let stellar_max_retries: u32 = match stellar_max_retries_raw.parse() {
            Ok(v) => v,
            Err(_) => {
                errors.push(format!(
                    "STELLAR_MAX_RETRIES must be a valid u32, got '{}'",
                    stellar_max_retries_raw
                ));
                3
            }
        };

        let cache_verification_ttl: u64 = match cache_verification_ttl_raw.parse() {
            Ok(v) => v,
            Err(_) => {
                errors.push(format!(
                    "CACHE_VERIFICATION_TTL must be a valid u64, got '{}'",
                    cache_verification_ttl_raw
                ));
                3600
            }
        };

        match Url::parse(&redis_url) {
            Ok(url) if matches!(url.scheme(), "redis" | "rediss") => {}
            Ok(_) | Err(_) => {
                errors.push(format!(
                    "REDIS_URL must be a valid redis:// or rediss:// URL, got '{}'",
                    redis_url
                ));
            }
        }

        if rate_limit_burst == 0 {
            errors.push("RATE_LIMIT_BURST must be greater than 0".to_string());
        }

        let webhook_urls: Vec<String> = webhook_urls_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|url| {
                if Url::parse(url).is_err() {
                    errors.push(format!("WEBHOOK_URLS must contain valid URLs, got '{}'", url));
                }
                url.to_string()
            })
            .collect();

        if !errors.is_empty() {
            if let Some(ref m) = metrics {
                m.increment_config_validation_failure();
            }
            let joined = errors.join("\n- ");
            return Err(ConfigError::Validation(format!("- {}", joined)));
        }

        // Successful load
        if let Some(ref m) = metrics {
            m.increment_config_reload();
        }

        Ok(Self {
            port,
            stellar_horizon_url,
            stellar_secret_key,
            redis_url,
            rate_limit_per_second,
            rate_limit_burst,
            stellar_max_retries,
            log_level,
            webhook_urls,
            webhook_secret,
            cache_verification_ttl,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        let keys = [
            "PORT",
            "STELLAR_HORIZON_URL",
            "STELLAR_SECRET_KEY",
            "REDIS_URL",
            "RATE_LIMIT_PER_SECOND",
            "RATE_LIMIT_BURST",
            "STELLAR_MAX_RETRIES",
            "LOG_LEVEL",
            "WEBHOOK_URLS",
            "WEBHOOK_SECRET",
            "CACHE_VERIFICATION_TTL",
        ];
        for key in keys {
            env::remove_var(key);
        }
    }

    #[test]
    fn from_env_uses_defaults_when_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        env::set_var(
            "STELLAR_SECRET_KEY",
            "SBU2RRGLXH3E5CQHTD3ODLDF2BWDCYUSSBLLZ5GNW7JXHDIYKXZWHOKR",
        );
        let cfg = AppConfig::from_env().expect("config should load with defaults");

        assert_eq!(cfg.port, 8080);
        assert_eq!(
            cfg.stellar_horizon_url,
            "https://horizon-testnet.stellar.org"
        );
        assert_eq!(cfg.redis_url, "redis://127.0.0.1:6379");
        assert_eq!(cfg.rate_limit_per_second, 10);
        assert_eq!(cfg.cache_verification_ttl, 3600);
    }

    #[test]
    fn from_env_invalid_values_report_errors() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        env::set_var("PORT", "0");
        env::set_var("STELLAR_HORIZON_URL", "not-a-url");
        env::set_var("REDIS_URL", "not-a-url");
        env::set_var("RATE_LIMIT_PER_SECOND", "0");
        env::set_var("RATE_LIMIT_BURST", "0");
        env::set_var("WEBHOOK_URLS", "https://ok.example.com, not-a-url");
        env::set_var(
            "STELLAR_SECRET_KEY",
            "SBU2RRGLXH3E5CQHTD3ODLDF2BWDCYUSSBLLZ5GNW7JXHDIYKXZWHOKR",
        );

        let err = AppConfig::from_env().expect_err("config should fail");
        let msg = err.to_string();

        assert!(msg.contains("PORT must be between 1 and 65535"));
        assert!(msg.contains("STELLAR_HORIZON_URL must be a valid URL"));
        assert!(msg.contains("REDIS_URL must be a valid redis:// or rediss:// URL"));
        assert!(msg.contains("RATE_LIMIT_PER_SECOND must be greater than 0"));
        assert!(msg.contains("RATE_LIMIT_BURST must be greater than 0"));
        assert!(msg.contains("WEBHOOK_URLS must contain valid URLs"));
    }

    #[test]
    fn from_env_rejects_invalid_stellar_secret_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        env::set_var(
            "STELLAR_SECRET_KEY",
            "SAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        );

        let err = AppConfig::from_env().expect_err("config should fail");
        let msg = err.to_string();

        assert!(msg.contains("STELLAR_SECRET_KEY must be a valid Stellar ed25519 secret key"));
    }

    #[test]
    fn from_env_parses_valid_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        env::set_var("PORT", "9090");
        env::set_var("STELLAR_HORIZON_URL", "https://example.com");
        env::set_var("REDIS_URL", "redis://redis:6379");
        env::set_var("RATE_LIMIT_PER_SECOND", "100");
        env::set_var("RATE_LIMIT_BURST", "100");
        env::set_var("WEBHOOK_URLS", "https://a.com, https://b.com");
        env::set_var(
            "STELLAR_SECRET_KEY",
            "SBU2RRGLXH3E5CQHTD3ODLDF2BWDCYUSSBLLZ5GNW7JXHDIYKXZWHOKR",
        );

        let cfg = AppConfig::from_env().expect("config should load");

        assert_eq!(cfg.port, 9090);
        assert_eq!(cfg.stellar_horizon_url, "https://example.com");
        assert_eq!(cfg.redis_url, "redis://redis:6379");
        assert_eq!(cfg.rate_limit_per_second, 100);
        assert_eq!(cfg.rate_limit_burst, 100);
        assert_eq!(cfg.webhook_urls.len(), 2);
    }

    #[test]
    fn debug_redacts_secret_values() {
        let config = AppConfig {
            port: 8080,
            stellar_horizon_url: "https://example.com".to_string(),
            stellar_secret_key: Some("secret-value".to_string()),
            redis_url: "redis://redis:6379".to_string(),
            rate_limit_per_second: 10,
            rate_limit_burst: 10,
            stellar_max_retries: 3,
            log_level: "info".to_string(),
            webhook_urls: vec!["https://webhook.example.com".to_string()],
            webhook_secret: Some("another-secret".to_string()),
            cache_verification_ttl: 3600,
        };

        let debug = format!("{:?}", config);
        assert!(!debug.contains("secret-value"));
        assert!(!debug.contains("another-secret"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn from_env_records_config_validation_failure() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        env::set_var("PORT", "0");
        env::set_var(
            "STELLAR_SECRET_KEY",
            "SBU2RRGLXH3E5CQHTD3ODLDF2BWDCYUSSBLLZ5GNW7JXHDIYKXZWHOKR",
        );

        let metrics = MetricsRegistry::arc();
        let _err = AppConfig::from_env_with_metrics(Some(Arc::clone(&metrics))).expect_err("should fail");

        let output = metrics.render();
        assert!(output.contains("config_validation_failures_total"));
    }

    #[test]
    fn from_env_records_config_reload_on_success() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        env::set_var(
            "STELLAR_SECRET_KEY",
            "SBU2RRGLXH3E5CQHTD3ODLDF2BWDCYUSSBLLZ5GNW7JXHDIYKXZWHOKR",
        );

        let metrics = MetricsRegistry::arc();
        let _cfg = AppConfig::from_env_with_metrics(Some(Arc::clone(&metrics))).expect("should succeed");

        let output = metrics.render();
        assert!(output.contains("config_reload_total"));
    }
}
