use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{
    boxed::Box,
    fmt,
    future::Future,
    string::{String, ToString},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
    vec::Vec,
};
use thiserror::Error;

use crate::{cache::CacheKey, config::AppConfig, rate_limit::StellarRateLimiter};

const DEFAULT_RETRY_BASE_DELAY_MS: u64 = 100;
const DEFAULT_RETRY_MAX_DELAY_MS: u64 = 10_000;
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_FAILURE_THRESHOLD: u32 = 5;
const DEFAULT_OPEN_DURATION_MS: u64 = 30_000;
const DEFAULT_HALF_OPEN_MAX_CALLS: u32 = 1;

#[derive(Debug, Clone)]
pub struct StellarClient {
    horizon_url: String,
    http_client: reqwest::Client,
    rate_limiter: Arc<StellarRateLimiter>,
    circuit_breaker: Arc<CircuitBreaker>,
    config: StellarClientConfig,
}

#[derive(Debug, Clone)]
pub struct StellarClientConfig {
    pub retry: RetryPolicy,
    pub circuit_breaker: CircuitBreakerConfig,
    pub request_timeout: Duration,
    pub rate_limit_per_second: u32,
    pub rate_limit_burst: u32,
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub jitter: RetryJitter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryJitter {
    None,
    Full,
}

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub open_duration: Duration,
    pub half_open_max_calls: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CircuitBreakerMetrics {
    pub trips: u64,
    pub recoveries: u64,
    pub half_open_successes: u64,
    pub half_open_failures: u64,
    pub rejected_calls: u64,
    pub successful_calls: u64,
    pub failed_calls: u64,
}

#[derive(Debug, Error)]
pub enum StellarError {
    #[error("stellar circuit breaker is {state:?}; retry after {retry_after:?}")]
    CircuitOpen {
        state: CircuitState,
        retry_after: Duration,
    },
    #[error("stellar request to {url} failed: {source}")]
    Request { url: String, source: String },
    #[error("stellar request to {url} timed out after {timeout:?}")]
    Timeout { url: String, timeout: Duration },
    #[error("stellar horizon returned HTTP {status} for {url}: {body}")]
    RetryableHttpStatus {
        status: u16,
        url: String,
        body: String,
    },
    #[error("stellar horizon returned non-retryable HTTP {status} for {url}: {body}")]
    NonRetryableHttpStatus {
        status: u16,
        url: String,
        body: String,
    },
    #[error("stellar horizon response for {url} could not be parsed: {0}. Body: {1}")]
    ResponseParse {
        url: String,
        body: String,
        source: serde_json::Error,
    },
    #[error("stellar verification for hash {hash} did not find a matching transaction")]
    VerificationNotFound { hash: String },
    #[error("stellar operation '{operation}' failed after {attempts} attempts ({retries_attempted} retries): {final_error}")]
    RetryExhausted {
        operation: &'static str,
        attempts: u32,
        retries_attempted: u32,
        final_error: Box<StellarError>,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TransactionRecord {
    pub transaction_id: String,
    pub timestamp: i64,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationResult {
    pub verified: bool,
    pub transaction_id: Option<String>,
    pub timestamp: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct HorizonTransactionsResponse {
    #[serde(rename = "_embedded")]
    embedded: HorizonEmbeddedRecords,
}

#[derive(Debug, Deserialize)]
struct HorizonEmbeddedRecords {
    records: Vec<HorizonTransactionRecord>,
}

#[derive(Debug, Deserialize)]
struct HorizonTransactionRecord {
    hash: Option<String>,
    created_at: Option<String>,
}

pub type StellarResult<T> = Result<T, StellarError>;

impl StellarClient {
    pub fn new(horizon_url: &str) -> Self {
        Self::with_config(horizon_url, StellarClientConfig::default())
    }

    pub fn from_app_config(config: &AppConfig) -> Self {
        Self::with_config(
            &config.stellar_horizon_url,
            StellarClientConfig::from_app_config(config),
        )
    }

    pub fn with_config(horizon_url: &str, config: StellarClientConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            horizon_url: trim_trailing_slash(horizon_url),
            http_client,
            rate_limiter: Arc::new(StellarRateLimiter::new(
                config.rate_limit_per_second,
                config.rate_limit_burst,
            )),
            circuit_breaker: Arc::new(CircuitBreaker::new(config.circuit_breaker.clone())),
            config,
        }
    }

    pub fn config(&self) -> &StellarClientConfig {
        &self.config
    }

    pub fn verification_cache_key(hash: &str) -> CacheKey {
        CacheKey::Verification(hash.to_string())
    }

    pub fn circuit_state(&self) -> CircuitState {
        self.circuit_breaker.state()
    }

    pub fn circuit_breaker_metrics(&self) -> CircuitBreakerMetrics {
        self.circuit_breaker.metrics()
    }

    pub async fn check_connection(&self) -> bool {
        self.http_client
            .get(&self.horizon_url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    pub async fn check_connection_with_retry(&self) -> StellarResult<bool> {
        let max_attempts = self.config.retry.max_retries.saturating_add(1);
        let mut last_error = None;

        for attempt in 0..max_attempts {
            match self.check_connection().await {
                true => return Ok(true),
                false => {
                    if attempt + 1 == max_attempts {
                        break;
                    }
                    last_error = Some(StellarError::Request {
                        url: self.horizon_url.clone(),
                        source: "connection check returned a non-success response".to_string(),
                    });
                    sleep(self.retry_delay(attempt)).await;
                }
            }
        }

        Err(StellarError::RetryExhausted {
            operation: "check_connection",
            attempts: max_attempts,
            retries_attempted: self.config.retry.max_retries,
            final_error: Box::new(last_error.unwrap_or_else(|| StellarError::Request {
                url: self.horizon_url.clone(),
                source: "connection check failed".to_string(),
            })),
        })
    }

    pub async fn verify_hash(&self, hash: &str) -> StellarResult<VerificationResult> {
        self.rate_limiter.acquire().await;
        let url = format!("{}/transactions?memo={}", self.horizon_url, hash);
        let resp = self.http_client.get(&url).send().await.map_err(|source| {
            if source.is_timeout() {
                StellarError::Timeout {
                    url: url.clone(),
                    timeout: self.config.request_timeout,
                }
            } else {
                StellarError::Request {
                    url: url.clone(),
                    source: source.to_string(),
                }
            }
        })?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if status.is_success() {
            let parsed: HorizonTransactionsResponse = serde_json::from_str(&body).map_err(|source| {
                StellarError::ResponseParse {
                    url: url.clone(),
                    body: truncate_body(&body),
                    source,
                }
            })?;
            let record = parsed.embedded.records.into_iter().next();

            match record {
                Some(record) => Ok(VerificationResult {
                    verified: true,
                    transaction_id: record.hash.filter(|value| !value.is_empty()),
                    timestamp: record
                        .created_at
                        .as_deref()
                        .and_then(parse_horizon_timestamp),
                }),
                None => Ok(VerificationResult {
                    verified: false,
                    transaction_id: None,
                    timestamp: None,
                }),
            }
        } else if is_retryable_status(status.as_u16()) {
            Err(StellarError::RetryableHttpStatus {
                status: status.as_u16(),
                url,
                body: truncate_body(&body),
            })
        } else {
            Err(StellarError::NonRetryableHttpStatus {
                status: status.as_u16(),
                url,
                body: truncate_body(&body),
            })
        }
    }

    pub async fn verify_hash_with_retry(&self, hash: &str) -> StellarResult<VerificationResult> {
        self.circuit_breaker
            .call(|| async { self.execute_verify_hash_with_retry(hash).await })
            .await
    }

    async fn execute_verify_hash_with_retry(&self, hash: &str) -> StellarResult<VerificationResult> {
        let max_attempts = self.config.retry.max_retries.saturating_add(1);
        let mut last_error = None;
        let mut attempts_made = 0;

        for attempt in 0..max_attempts {
            attempts_made = attempt + 1;
            match self.verify_hash(hash).await {
                Ok(result) => return Ok(result),
                Err(err) if err.is_retryable() && attempt + 1 < max_attempts => {
                    last_error = Some(err);
                    sleep(self.retry_delay(attempt)).await;
                }
                Err(err) => {
                    last_error = Some(err);
                    break;
                }
            }
        }

        let final_error = last_error.unwrap_or_else(|| StellarError::Request {
            url: format!("{}/transactions?memo={}", self.horizon_url, hash),
            source: "verification failed without a recorded error".to_string(),
        });
        let retries_attempted = attempts_made.saturating_sub(1);

        Err(StellarError::RetryExhausted {
            operation: "verify_hash",
            attempts: attempts_made,
            retries_attempted,
            final_error: Box::new(final_error),
        })
    }

    fn retry_delay(&self, attempt: u32) -> Duration {
        let multiplier = 2_u32.saturating_pow(attempt.min(31));
        let exponential = self
            .config
            .retry
            .base_delay
            .checked_mul(multiplier)
            .unwrap_or(self.config.retry.max_delay);
        let capped = exponential.min(self.config.retry.max_delay);

        match self.config.retry.jitter {
            RetryJitter::None => capped,
            RetryJitter::Full => jittered_delay(capped),
        }
    }

    pub async fn anchor_transfer(&self, _transfer_hash: &str, _memo: &str) -> StellarResult<()> {
        Ok(())
    }
}

impl StellarClientConfig {
    pub fn from_app_config(config: &AppConfig) -> Self {
        Self {
            retry: RetryPolicy {
                max_retries: config.stellar_max_retries,
                base_delay: Duration::from_millis(config.stellar_retry_base_delay_ms),
                max_delay: Duration::from_millis(config.stellar_retry_max_delay_ms),
                jitter: if config.stellar_retry_jitter_enabled {
                    RetryJitter::Full
                } else {
                    RetryJitter::None
                },
            },
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: config.stellar_circuit_breaker_failure_threshold,
                open_duration: Duration::from_millis(config.stellar_circuit_breaker_open_duration_ms),
                half_open_max_calls: config.stellar_circuit_breaker_half_open_max_calls,
            },
            request_timeout: Duration::from_millis(config.stellar_request_timeout_ms),
            rate_limit_per_second: config.rate_limit_per_second,
            rate_limit_burst: config.rate_limit_burst,
        }
    }
}

impl Default for StellarClientConfig {
    fn default() -> Self {
        Self {
            retry: RetryPolicy {
                max_retries: 3,
                base_delay: Duration::from_millis(DEFAULT_RETRY_BASE_DELAY_MS),
                max_delay: Duration::from_millis(DEFAULT_RETRY_MAX_DELAY_MS),
                jitter: RetryJitter::Full,
            },
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: DEFAULT_FAILURE_THRESHOLD,
                open_duration: Duration::from_millis(DEFAULT_OPEN_DURATION_MS),
                half_open_max_calls: DEFAULT_HALF_OPEN_MAX_CALLS,
            },
            request_timeout: Duration::from_millis(DEFAULT_REQUEST_TIMEOUT_MS),
            rate_limit_per_second: 10,
            rate_limit_burst: 10,
        }
    }
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            open_duration: Duration::from_millis(DEFAULT_OPEN_DURATION_MS),
            half_open_max_calls: DEFAULT_HALF_OPEN_MAX_CALLS,
        }
    }
}

impl StellarError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::CircuitOpen { .. }
            | Self::Request { .. }
            | Self::Timeout { .. }
            | Self::RetryableHttpStatus { .. }
            | Self::ResponseParse { .. } => true,
            Self::RetryExhausted { final_error, .. } => final_error.is_retryable(),
            Self::NonRetryableHttpStatus { .. }
            | Self::VerificationNotFound { .. } => false,
        }
    }

    pub fn affects_circuit_breaker(&self) -> bool {
        match self {
            Self::RetryExhausted { final_error, .. } => final_error.affects_circuit_breaker(),
            Self::Request { .. } | Self::Timeout { .. } | Self::RetryableHttpStatus { .. } => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Mutex<CircuitState>,
    opened_at: Mutex<Option<Instant>>,
    consecutive_failures: Mutex<u32>,
    half_open_in_flight: Mutex<u32>,
    half_open_successes: Mutex<u32>,
    metrics: Mutex<CircuitBreakerMetrics>,
}

impl CircuitBreaker {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: Mutex::new(CircuitState::Closed),
            opened_at: Mutex::new(None),
            consecutive_failures: Mutex::new(0),
            half_open_in_flight: Mutex::new(0),
            half_open_successes: Mutex::new(0),
            metrics: Mutex::new(CircuitBreakerMetrics::default()),
        }
    }

    fn state(&self) -> CircuitState {
        *self.state.lock().unwrap()
    }

    fn metrics(&self) -> CircuitBreakerMetrics {
        *self.metrics.lock().unwrap()
    }

    async fn call<F, Fut, T>(&self, operation: F) -> StellarResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = StellarResult<T>>,
    {
        self.allow_call()?;
        let result = operation().await;
        self.record_result(&result);
        result
    }

    fn allow_call(&self) -> StellarResult<()> {
        let mut state = self.state.lock().unwrap();

        match *state {
            CircuitState::Closed => Ok(()),
            CircuitState::Open => {
                let opened_at = *self.opened_at.lock().unwrap();
                if let Some(opened_at) = opened_at {
                    let elapsed = opened_at.elapsed();
                    if elapsed >= self.config.open_duration {
                        *state = CircuitState::HalfOpen;
                        *self.half_open_in_flight.lock().unwrap() = 0;
                        *self.half_open_successes.lock().unwrap() = 0;
                        return Ok(());
                    }

                    let retry_after = self.config.open_duration.saturating_sub(elapsed);
                    self.increment_rejected_calls();
                    return Err(StellarError::CircuitOpen {
                        state: *state,
                        retry_after,
                    });
                }

                *state = CircuitState::HalfOpen;
                Ok(())
            }
            CircuitState::HalfOpen => {
                let max_calls = self.config.half_open_max_calls.max(1);
                let mut in_flight = self.half_open_in_flight.lock().unwrap();
                if *in_flight >= max_calls {
                    self.increment_rejected_calls();
                    Err(StellarError::CircuitOpen {
                        state: *state,
                        retry_after: Duration::ZERO,
                    })
                } else {
                    *in_flight += 1;
                    Ok(())
                }
            }
        }
    }

    fn record_result<T>(&self, result: &StellarResult<T>) {
        match result {
            Ok(_) => self.record_success(),
            Err(err) if err.affects_circuit_breaker() => self.record_failure(),
            Err(_) => {}
        }
    }

    fn record_success(&self) {
        let state = *self.state.lock().unwrap();

        match state {
            CircuitState::Closed => {
                *self.consecutive_failures.lock().unwrap() = 0;
                self.increment_successful_calls();
            }
            CircuitState::HalfOpen => {
                {
                    let mut in_flight = self.half_open_in_flight.lock().unwrap();
                    *in_flight = in_flight.saturating_sub(1);
                }

                let should_close;
                {
                    let mut successes = self.half_open_successes.lock().unwrap();
                    *successes = successes.saturating_add(1);
                    should_close = *successes >= self.config.half_open_max_calls.max(1);
                }
                {
                    let mut metrics = self.metrics.lock().unwrap();
                    metrics.half_open_successes = metrics.half_open_successes.saturating_add(1);
                }
                self.increment_successful_calls();

                if should_close {
                    *self.state.lock().unwrap() = CircuitState::Closed;
                    *self.opened_at.lock().unwrap() = None;
                    *self.consecutive_failures.lock().unwrap() = 0;
                    self.increment_recoveries();
                }
            }
            CircuitState::Open => {}
        }
    }

    fn record_failure(&self) {
        let state = *self.state.lock().unwrap();

        match state {
            CircuitState::Closed => {
                let mut failures = self.consecutive_failures.lock().unwrap();
                *failures = failures.saturating_add(1);
                self.increment_failed_calls();

                if *failures >= self.config.failure_threshold.max(1) {
                    *self.state.lock().unwrap() = CircuitState::Open;
                    *self.opened_at.lock().unwrap() = Some(Instant::now());
                    self.increment_trips();
                }
            }
            CircuitState::HalfOpen => {
                {
                    let mut in_flight = self.half_open_in_flight.lock().unwrap();
                    *in_flight = in_flight.saturating_sub(1);
                }
                {
                    let mut metrics = self.metrics.lock().unwrap();
                    metrics.half_open_failures = metrics.half_open_failures.saturating_add(1);
                }
                self.increment_failed_calls();
                *self.state.lock().unwrap() = CircuitState::Open;
                *self.opened_at.lock().unwrap() = Some(Instant::now());
                self.increment_trips();
            }
            CircuitState::Open => {}
        }
    }

    fn increment_trips(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.trips = metrics.trips.saturating_add(1);
    }

    fn increment_recoveries(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.recoveries = metrics.recoveries.saturating_add(1);
    }

    fn increment_rejected_calls(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.rejected_calls = metrics.rejected_calls.saturating_add(1);
    }

    fn increment_successful_calls(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.successful_calls = metrics.successful_calls.saturating_add(1);
    }

    fn increment_failed_calls(&self) {
        let mut metrics = self.metrics.lock().unwrap();
        metrics.failed_calls = metrics.failed_calls.saturating_add(1);
    }
}

impl fmt::Display for CircuitState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half-open"),
        }
    }
}

fn trim_trailing_slash(value: &str) -> String {
    value.trim_end_matches('/').to_string()
}

fn is_retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || (500..=599).contains(&status)
}

fn truncate_body(body: &str) -> String {
    const MAX_BODY_CHARS: usize = 512;
    if body.chars().count() <= MAX_BODY_CHARS {
        body.to_string()
    } else {
        format!("{}...", body.chars().take(MAX_BODY_CHARS).collect::<String>())
    }
}

fn jittered_delay(max_delay: Duration) -> Duration {
    let millis = (max_delay.as_secs_f64() * 1000.0 * rand::thread_rng().gen::<f64>()).round()
        as u64;
    Duration::from_millis(millis)
}

fn parse_horizon_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp())
}

async fn sleep(delay: Duration) {
    if delay.is_zero() {
        tokio::task::yield_now().await;
    } else {
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config() -> StellarClientConfig {
        StellarClientConfig {
            retry: RetryPolicy {
                max_retries: 3,
                base_delay: Duration::ZERO,
                max_delay: Duration::from_millis(1),
                jitter: RetryJitter::None,
            },
            circuit_breaker: CircuitBreakerConfig {
                failure_threshold: 5,
                open_duration: Duration::from_millis(20),
                half_open_max_calls: 1,
            },
            request_timeout: Duration::from_secs(2),
            rate_limit_per_second: 100,
            rate_limit_burst: 100,
        }
    }

    fn horizon_success_body() -> serde_json::Value {
        json!({
            "_embedded": {
                "records": [
                    {
                        "hash": "transaction-id",
                        "created_at": "2024-01-01T00:00:00Z"
                    }
                ]
            }
        })
    }

    #[tokio::test]
    async fn verify_hash_with_retry_succeeds_after_transient_failures() {
        let server = MockServer::start().await;
        let hash = "document-hash";

        Mock::given(method("GET"))
            .and(path("/transactions"))
            .and(query_param("memo", hash))
            .respond_with(ResponseTemplate::new(503))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/transactions"))
            .and(query_param("memo", hash))
            .respond_with(ResponseTemplate::new(200).set_body_json(horizon_success_body()))
            .expect(1)
            .mount(&server)
            .await;

        let client = StellarClient::with_config(&server.uri(), test_config());
        let result = client.verify_hash_with_retry(hash).await.unwrap();

        assert!(result.verified);
        assert_eq!(result.transaction_id.as_deref(), Some("transaction-id"));
        assert_eq!(result.timestamp, Some(1_704_067_200));
        assert_eq!(client.circuit_breaker_metrics().successful_calls, 1);
    }

    #[tokio::test]
    async fn verify_hash_with_retry_reports_attempts_and_final_error() {
        let server = MockServer::start().await;
        let hash = "document-hash";
        let mut config = test_config();
        config.retry.max_retries = 1;
        config.circuit_breaker.failure_threshold = 1;

        Mock::given(method("GET"))
            .and(path("/transactions"))
            .and(query_param("memo", hash))
            .respond_with(ResponseTemplate::new(503))
            .expect(2)
            .mount(&server)
            .await;

        let client = StellarClient::with_config(&server.uri(), config);
        let err = client.verify_hash_with_retry(hash).await.unwrap_err();

        match err {
            StellarError::RetryExhausted {
                attempts,
                retries_attempted,
                final_error,
                ..
            } => {
                assert_eq!(attempts, 2);
                assert_eq!(retries_attempted, 1);
                assert!(final_error.to_string().contains("HTTP 503"));
            }
            other => panic!("expected RetryExhausted, got {other:?}"),
        }

        assert_eq!(client.circuit_state(), CircuitState::Open);
        assert_eq!(client.circuit_breaker_metrics().trips, 1);
    }

    #[tokio::test]
    async fn circuit_breaker_rejects_calls_while_open() {
        let server = MockServer::start().await;
        let hash = "document-hash";
        let mut config = test_config();
        config.retry.max_retries = 0;
        config.circuit_breaker.failure_threshold = 1;
        config.circuit_breaker.open_duration = Duration::from_millis(50);

        Mock::given(method("GET"))
            .and(path("/transactions"))
            .and(query_param("memo", hash))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let client = StellarClient::with_config(&server.uri(), config);
        let _ = client.verify_hash_with_retry(hash).await.unwrap_err();

        assert_eq!(client.circuit_state(), CircuitState::Open);

        let err = client.verify_hash_with_retry(hash).await.unwrap_err();
        match err {
            StellarError::CircuitOpen {
                state,
                retry_after,
            } => {
                assert_eq!(state, CircuitState::Open);
                assert!(retry_after <= Duration::from_millis(50));
            }
            other => panic!("expected CircuitOpen, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn circuit_breaker_recovers_from_half_open_success() {
        let server = MockServer::start().await;
        let hash = "document-hash";
        let mut config = test_config();
        config.retry.max_retries = 0;
        config.circuit_breaker.failure_threshold = 1;
        config.circuit_breaker.open_duration = Duration::from_millis(10);

        Mock::given(method("GET"))
            .and(path("/transactions"))
            .and(query_param("memo", hash))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/transactions"))
            .and(query_param("memo", hash))
            .respond_with(ResponseTemplate::new(200).set_body_json(horizon_success_body()))
            .expect(1)
            .mount(&server)
            .await;

        let client = StellarClient::with_config(&server.uri(), config);
        let _ = client.verify_hash_with_retry(hash).await.unwrap_err();
        tokio::time::sleep(Duration::from_millis(15)).await;
        let result = client.verify_hash_with_retry(hash).await.unwrap();

        assert!(result.verified);
        assert_eq!(client.circuit_state(), CircuitState::Closed);
        assert_eq!(client.circuit_breaker_metrics().recoveries, 1);
        assert_eq!(client.circuit_breaker_metrics().half_open_successes, 1);
    }

    #[test]
    fn retry_delay_uses_exponential_backoff_without_jitter() {
        let client = StellarClient::with_config(
            "https://horizon-testnet.stellar.org",
            StellarClientConfig {
                retry: RetryPolicy {
                    max_retries: 3,
                    base_delay: Duration::from_millis(100),
                    max_delay: Duration::from_secs(10),
                    jitter: RetryJitter::None,
                },
                ..StellarClientConfig::default()
            },
        );

        assert_eq!(client.retry_delay(0), Duration::from_millis(100));
        assert_eq!(client.retry_delay(1), Duration::from_millis(200));
        assert_eq!(client.retry_delay(2), Duration::from_millis(400));
    }
}
