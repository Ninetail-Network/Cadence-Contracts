use std::prelude::v1::*;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::cache::CacheKey;
use crate::metrics::MetricsRegistry;

#[derive(Clone)]
pub struct StellarClient {
    horizon_url: String,
    http_client: reqwest::Client,
    max_retries: u32,
    metrics: Option<Arc<MetricsRegistry>>,
}

impl std::fmt::Debug for StellarClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StellarClient")
            .field("horizon_url", &self.horizon_url)
            .field("http_client", &self.http_client)
            .field("max_retries", &self.max_retries)
            .field("metrics", &self.metrics.as_ref().map(|_| "<metrics>"))
            .finish()
    }
}

/// Categorised outcome of a Stellar Horizon verification.
///
/// Distinguishes the four states required by the acceptance criteria:
/// confirmed match, no match, network failure, and malformed response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationStatus {
    /// A matching Stellar transaction was found with correct memo.
    ConfirmedMatch,
    /// Horizon was reachable but no transaction matched the hash.
    NoMatch,
    /// All retries exhausted due to network / connection errors.
    NetworkError,
    /// Horizon returned a response that could not be parsed.
    MalformedResponse,
}

/// Result of a Stellar Horizon verification request.
///
/// When `status` is [`VerificationStatus::ConfirmedMatch`], `transaction_id`
/// and `timestamp` carry the on-chain proof. For all other statuses both
/// fields are `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub status: VerificationStatus,
    pub transaction_id: Option<String>,
    pub timestamp: Option<i64>,
}

impl VerificationResult {
    /// Convenience: `true` only for [`VerificationStatus::ConfirmedMatch`].
    pub fn verified(&self) -> bool {
        matches!(self.status, VerificationStatus::ConfirmedMatch)
    }
}

/// A parsed Stellar transaction extracted from a Horizon response.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TransactionRecord {
    pub transaction_id: String,
    pub timestamp: i64,
    /// Always `true` when constructed from a confirmed match.
    /// Retained for cache backward-compatibility; new code should
    /// use [`VerificationStatus::ConfirmedMatch`] instead.
    pub verified: bool,
}

impl StellarClient {
    pub fn new(horizon_url: &str) -> Self {
        Self {
            horizon_url: horizon_url.to_string(),
            http_client: reqwest::Client::new(),
            max_retries: 3,
            metrics: None,
        }
    }

    /// Set the maximum number of retries for `verify_hash`.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_metrics(mut self, metrics: Arc<MetricsRegistry>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn verification_cache_key(hash: &str) -> CacheKey {
        CacheKey::verification(hash)
    }

    pub async fn check_connection(&self) -> bool {
        let start = MetricsRegistry::start_timer();
        let result = self
            .http_client
            .get(&self.horizon_url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);

        if let Some(ref m) = self.metrics {
            m.increment_request_count();
            let status = if result { "success" } else { "error" };
            m.record_horizon_latency(status, MetricsRegistry::elapsed_secs(start));
            if !result {
                m.increment_error_count();
            }
        }
        result
    }

    /// Verify a document hash against Stellar Horizon with retries.
    ///
    /// Queries `GET /transactions?memo={hash}`, parses the response, and:
    ///
    /// * Cross-checks the returned transaction's memo field against the
    ///   requested hash.
    /// * Extracts the transaction ID and ledger close timestamp.
    /// * Distinguishes between [`VerificationStatus::ConfirmedMatch`],
    ///   [`VerificationStatus::NoMatch`], [`VerificationStatus::NetworkError`],
    ///   and [`VerificationStatus::MalformedResponse`].
    ///
    /// Records latency, success/failure, and retry metrics.
    pub async fn verify_hash(&self, hash: &str) -> VerificationResult {
        let overall_start = MetricsRegistry::start_timer();
        let mut last_status = VerificationStatus::NoMatch;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                if let Some(ref m) = self.metrics {
                    m.increment_retry();
                }
                tokio::time::sleep(Duration::from_millis(200 * attempt as u64)).await;
            }

            let horizon_start = MetricsRegistry::start_timer();
            let url = format!("{}/transactions?memo={}", self.horizon_url, hash);
            let resp_result = self.http_client.get(&url).send().await;

            match resp_result {
                Ok(resp) => {
                    let horizon_latency = MetricsRegistry::elapsed_secs(horizon_start);
                    let status_str = if resp.status().is_success() {
                        "success"
                    } else {
                        "error"
                    };

                    if let Some(ref m) = self.metrics {
                        m.record_horizon_latency(status_str, horizon_latency);
                    }

                    if resp.status().is_success() {
                        match self.parse_horizon_transaction(resp, hash).await {
                            Ok(Some(record)) => {
                                if let Some(ref m) = self.metrics {
                                    m.record_verification(
                                        "success",
                                        MetricsRegistry::elapsed_secs(overall_start),
                                    );
                                }
                                return VerificationResult {
                                    status: VerificationStatus::ConfirmedMatch,
                                    transaction_id: Some(record.transaction_id),
                                    timestamp: Some(record.timestamp),
                                };
                            }
                            Ok(None) => {
                                last_status = VerificationStatus::NoMatch;
                                break; // legitimate negative — don't retry
                            }
                            Err(_) => {
                                last_status = VerificationStatus::MalformedResponse;
                                // parse failure may be transient — continue retry
                            }
                        }
                    } else {
                        last_status = VerificationStatus::NetworkError;
                        // HTTP error — continue retry
                    }
                }
                Err(_) => {
                    let horizon_latency = MetricsRegistry::elapsed_secs(horizon_start);
                    if let Some(ref m) = self.metrics {
                        m.record_horizon_latency("error", horizon_latency);
                    }
                    last_status = VerificationStatus::NetworkError;
                    // Network error — continue retry
                }
            }
        }

        if let Some(ref m) = self.metrics {
            m.record_verification("failure", MetricsRegistry::elapsed_secs(overall_start));
        }
        VerificationResult {
            status: last_status,
            transaction_id: None,
            timestamp: None,
        }
    }

    /// Parse a Horizon `/transactions` response and cross-check the memo
    /// field against the expected hash.
    async fn parse_horizon_transaction(
        &self,
        resp: reqwest::Response,
        expected_hash: &str,
    ) -> Result<Option<TransactionRecord>> {
        #[derive(Deserialize)]
        struct HorizonEmbedded {
            records: Vec<HorizonTransaction>,
        }

        #[derive(Deserialize)]
        struct HorizonResponse {
            #[serde(rename = "_embedded")]
            embedded: HorizonEmbedded,
        }

        #[derive(Deserialize)]
        struct HorizonTransaction {
            id: String,
            created_at: Option<String>,
            #[serde(default)]
            memo: Option<String>,
            #[serde(rename = "memo_type", default)]
            memo_type: Option<String>,
        }

        let body: HorizonResponse = resp.json().await?;

        for tx in body.embedded.records {
            // Cross-check: the transaction's memo must match the expected hash.
            // Horizon filters by memo on the server side, but we verify
            // client-side for defense in depth.
            // Only "text" memos are relevant — skip "hash", "return", etc.
            let memo_matches = tx.memo_type.as_deref() == Some("text")
                && tx
                    .memo
                    .as_deref()
                    .map(|m| m.to_lowercase() == expected_hash.to_lowercase())
                    .unwrap_or(false);

            if memo_matches {
                let timestamp = tx
                    .created_at
                    .as_ref()
                    .and_then(|ts| {
                        chrono::DateTime::parse_from_rfc3339(ts)
                            .ok()
                            .map(|dt| dt.timestamp())
                    })
                    .unwrap_or(0);

                return Ok(Some(TransactionRecord {
                    transaction_id: tx.id,
                    timestamp,
                    verified: true,
                }));
            }
        }

        Ok(None)
    }

    pub async fn anchor_transfer(&self, _transfer_hash: &str, _memo: &str) -> Result<()> {
        if let Some(ref m) = self.metrics {
            m.increment_request_count();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_accepts_optional_metrics() {
        let client = StellarClient::new("https://horizon-testnet.stellar.org");
        let metrics = MetricsRegistry::arc();
        let client = client.with_metrics(metrics);
        assert!(client.metrics.is_some());
    }

    #[test]
    fn verification_cache_key_is_consistent() {
        let key = StellarClient::verification_cache_key("abc123");
        assert_eq!(key.as_string(), "verification:abc123");
    }

    #[test]
    fn verification_cache_key_normalizes_uppercase() {
        let key = StellarClient::verification_cache_key("ABC123");
        assert_eq!(key.as_string(), "verification:abc123");
    }

    #[test]
    fn verification_result_verified_convenience() {
        let confirmed = VerificationResult {
            status: VerificationStatus::ConfirmedMatch,
            transaction_id: Some("tx123".into()),
            timestamp: Some(12345),
        };
        assert!(confirmed.verified());

        let no_match = VerificationResult {
            status: VerificationStatus::NoMatch,
            transaction_id: None,
            timestamp: None,
        };
        assert!(!no_match.verified());
    }

    /// ── Mocked Horizon tests ──────────────────────────────────────────
    ///
    /// These tests use `wiremock` to stand up a local HTTP server that
    /// simulates Horizon responses.  See `Cargo.toml` `[dev-dependencies]`.

    #[cfg(test)]
    mod horizon_mock {
        use super::*;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// Sample Horizon transaction JSON that matches a known hash.
        fn horizon_tx_json(id: &str, memo: &str, created_at: &str) -> serde_json::Value {
            serde_json::json!({
                "_embedded": {
                    "records": [{
                        "id": id,
                        "created_at": created_at,
                        "memo": memo,
                        "memo_type": "text"
                    }]
                }
            })
        }

        /// Empty Horizon response (no matching transactions).
        fn horizon_empty_json() -> serde_json::Value {
            serde_json::json!({
                "_embedded": {
                    "records": []
                }
            })
        }

        #[tokio::test]
        async fn verify_hash_returns_confirmed_match_when_memo_matches() {
            let server = MockServer::start().await;
            let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

            Mock::given(method("GET"))
                .and(path("transactions"))
                .and(query_param("memo", hash))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    horizon_tx_json(
                        "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                        hash,
                        "2024-01-15T10:30:00Z",
                    ),
                ))
                .mount(&server)
                .await;

            let client = StellarClient::new(&server.uri())
                .with_max_retries(0);

            let result = client.verify_hash(hash).await;

            assert_eq!(result.status, VerificationStatus::ConfirmedMatch);
            assert!(result.transaction_id.is_some());
            assert!(result.timestamp.unwrap() > 0);
        }

        #[tokio::test]
        async fn verify_hash_returns_no_match_when_horizon_empty() {
            let server = MockServer::start().await;
            let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

            Mock::given(method("GET"))
                .and(path("transactions"))
                .and(query_param("memo", hash))
                .respond_with(ResponseTemplate::new(200).set_body_json(horizon_empty_json()))
                .mount(&server)
                .await;

            let client = StellarClient::new(&server.uri())
                .with_max_retries(0);

            let result = client.verify_hash(hash).await;

            assert_eq!(result.status, VerificationStatus::NoMatch);
            assert!(!result.verified());
        }

        #[tokio::test]
        async fn verify_hash_returns_no_match_when_memo_mismatch() {
            let server = MockServer::start().await;
            let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

            // Horizon returns a transaction, but its memo doesn't match
            Mock::given(method("GET"))
                .and(path("transactions"))
                .and(query_param("memo", hash))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    horizon_tx_json(
                        "tx123",
                        "wrong-hash-0000000000000000000000000000000000000000000000000000",
                        "2024-01-15T10:30:00Z",
                    ),
                ))
                .mount(&server)
                .await;

            let client = StellarClient::new(&server.uri())
                .with_max_retries(0);

            let result = client.verify_hash(hash).await;

            assert_eq!(result.status, VerificationStatus::NoMatch);
            assert!(!result.verified());
        }

        #[tokio::test]
        async fn verify_hash_returns_network_error_on_http_500() {
            let server = MockServer::start().await;
            let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

            Mock::given(method("GET"))
                .and(path("transactions"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&server)
                .await;

            let client = StellarClient::new(&server.uri())
                .with_max_retries(0);

            let result = client.verify_hash(hash).await;

            assert_eq!(result.status, VerificationStatus::NetworkError);
            assert!(!result.verified());
        }

        #[tokio::test]
        async fn verify_hash_returns_malformed_response_for_invalid_json() {
            let server = MockServer::start().await;
            let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

            Mock::given(method("GET"))
                .and(path("transactions"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_string("not-valid-json{{{")
                )
                .mount(&server)
                .await;

            let client = StellarClient::new(&server.uri())
                .with_max_retries(0);

            let result = client.verify_hash(hash).await;

            assert_eq!(result.status, VerificationStatus::MalformedResponse);
            assert!(!result.verified());
        }

        #[tokio::test]
        async fn verify_hash_retries_on_transient_errors() {
            let server = MockServer::start().await;
            let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

            // First two attempts return 500, third succeeds
            Mock::given(method("GET"))
                .and(path("transactions"))
                .and(query_param("memo", hash))
                .respond_with(ResponseTemplate::new(500))
                .up_to_n_times(2)
                .expect(2)
                .mount(&server)
                .await;

            Mock::given(method("GET"))
                .and(path("transactions"))
                .and(query_param("memo", hash))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    horizon_tx_json("tx-retry-ok", hash, "2024-01-15T10:30:00Z"),
                ))
                .mount(&server)
                .await;

            let client = StellarClient::new(&server.uri())
                .with_max_retries(3)
                .with_metrics(MetricsRegistry::arc());

            let result = client.verify_hash(hash).await;

            assert_eq!(result.status, VerificationStatus::ConfirmedMatch);
            assert!(result.verified());
        }

        #[tokio::test]
        async fn verify_hash_exhausts_retries_on_persistent_errors() {
            let server = MockServer::start().await;
            let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

            Mock::given(method("GET"))
                .and(path("transactions"))
                .respond_with(ResponseTemplate::new(500))
                .expect(4) // initial + 3 retries
                .mount(&server)
                .await;

            let client = StellarClient::new(&server.uri())
                .with_max_retries(3);

            let result = client.verify_hash(hash).await;

            assert_eq!(result.status, VerificationStatus::NetworkError);
            assert!(!result.verified());
        }
    }
}
