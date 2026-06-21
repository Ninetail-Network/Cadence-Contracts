use std::num::NonZeroU32;
use std::prelude::v1::*;
use std::sync::Arc;

use governor::{Quota, RateLimiter};
use serde::{Deserialize, Serialize};

use crate::metrics::MetricsRegistry;

/// The default rate limiter type used by this crate, backed by the Quanta monotonic clock.
pub type DefaultRateLimiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::QuantaClock,
>;

/// Result of a rate limit check operation.
///
/// When a rate limit is exceeded, the `retry_after_secs` field indicates
/// how many seconds the caller should wait before retrying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitExceededInfo {
    /// Estimated seconds to wait before a token will be available
    pub retry_after_secs: u64,
}

impl RateLimitExceededInfo {
    /// Create a new rate limit exceeded info with the given retry-after duration.
    pub fn new(retry_after_secs: u64) -> Self {
        Self { retry_after_secs }
    }
}

/// Token bucket state for a single issuer or address.
///
/// Tracks tokens available and the last refill timestamp, enabling
/// token bucket algorithm implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBucketState {
    /// Number of tokens currently available (0.0 to max_burst)
    pub tokens: u64,
    /// Unix timestamp of last token refill
    pub last_refill_secs: u64,
}

impl TokenBucketState {
    /// Create a new token bucket in its fully charged state.
    pub fn new_full(max_burst: u64) -> Self {
        Self {
            tokens: max_burst,
            last_refill_secs: 0, // Will be set on first check
        }
    }

    /// Compute the number of tokens to refill based on elapsed time.
    ///
    /// Returns the new tokens available and updated timestamp, clamped to max_burst.
    pub fn refill(
        &self,
        elapsed_secs: u64,
        refill_rate: u64,
        max_burst: u64,
    ) -> (u64, u64) {
        let refilled = self.tokens + (elapsed_secs * refill_rate);
        (refilled.min(max_burst), elapsed_secs)
    }

    /// Attempt to consume `cost` tokens. Returns the updated state if successful,
    /// or the current state if insufficient tokens are available.
    pub fn try_consume(&self, cost: u64) -> Result<Self, Self> {
        if self.tokens >= cost {
            Ok(Self {
                tokens: self.tokens - cost,
                last_refill_secs: self.last_refill_secs,
            })
        } else {
            Err(*self)
        }
    }
}

/// A metrics-aware wrapper around `governor::RateLimiter`.
///
/// Consumes tokens with `until_ready()` (blocking) or `check()` (non-blocking)
/// and records token consumption and violation metrics.
pub struct MetricsRateLimiter {
    limiter: DefaultRateLimiter,
    metrics: Option<Arc<MetricsRegistry>>,
}

impl MetricsRateLimiter {
    /// Build a rate limiter with the given per-second rate and burst allowance,
    /// optionally wired to a metrics registry.
    pub fn new(per_second: u32, burst: u32, metrics: Option<Arc<MetricsRegistry>>) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(per_second.max(1)).unwrap())
            .allow_burst(NonZeroU32::new(burst.max(1)).unwrap());
        let limiter = RateLimiter::direct(quota);
        Self { limiter, metrics }
    }

    /// Non-blocking check: returns `Ok(())` if within limits, or a guard error.
    /// Records token consumption on success, violation on failure.
    pub fn check(&self) -> Result<(), governor::NotUntil<governor::clock::QuantaInstant>> {
        match self.limiter.check() {
            Ok(()) => {
                if let Some(ref m) = self.metrics {
                    m.record_token_consumed();
                }
                Ok(())
            }
            Err(negative) => {
                if let Some(ref m) = self.metrics {
                    m.increment_rate_limit_violation();
                }
                Err(negative)
            }
        }
    }

    /// Blocking wait until a token is available. Records token consumption.
    pub async fn until_ready(&self) {
        self.limiter.until_ready().await;
        if let Some(ref m) = self.metrics {
            m.record_token_consumed();
        }
    }

    /// Return a reference to the inner rate limiter for advanced use.
    pub fn inner(&self) -> &DefaultRateLimiter {
        &self.limiter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_rate_limiter_consumes_token_on_check() {
        let metrics = MetricsRegistry::arc();
        let rl = MetricsRateLimiter::new(100, 100, Some(Arc::clone(&metrics)));

        assert!(rl.check().is_ok());
        let output = metrics.render();
        assert!(output.contains("rate_limit_tokens_consumed_total"));
    }

    #[test]
    fn metrics_rate_limiter_records_violation_when_exhausted() {
        let metrics = MetricsRegistry::arc();
        // 1 per second, burst 0 — first consumes token, second should fail
        let rl = MetricsRateLimiter::new(1, 0, Some(Arc::clone(&metrics)));

        let _ = rl.check(); // consume the only token
        let result = rl.check();
        assert!(result.is_err());

        let output = metrics.render();
        assert!(output.contains("rate_limit_violations_total"));
    }

    #[test]
    fn build_rate_limiter_creates_valid_limiter() {
        let rl = build_rate_limiter(10, 10);
        assert!(rl.check().is_ok());
    }

    #[tokio::test]
    async fn until_ready_consumes_token() {
        let metrics = MetricsRegistry::arc();
        let rl = MetricsRateLimiter::new(100, 100, Some(Arc::clone(&metrics)));

        rl.until_ready().await;

        let output = metrics.render();
        assert!(output.contains("rate_limit_tokens_consumed_total"));
    }

    #[test]
    fn metrics_rate_limiter_without_metrics_does_not_panic() {
        let rl = MetricsRateLimiter::new(10, 10, None);
        assert!(rl.check().is_ok());
    }
}
