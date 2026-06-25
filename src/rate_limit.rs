use governor::{clock::{Clock, QuantaClock}, Quota, RateLimiter};
use std::{num::NonZeroU32, sync::Arc, time::Duration};

use crate::metrics::MetricsRegistry;

/// The default rate limiter type used by this crate, backed by the Quanta monotonic clock.
pub type DefaultRateLimiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::QuantaClock,
>;

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

/// Build a bare `governor::RateLimiter` without metrics (legacy compatibility).
pub fn build_rate_limiter(per_second: u32, burst: u32) -> DefaultRateLimiter {
    let quota = Quota::per_second(NonZeroU32::new(per_second.max(1)).unwrap())
        .allow_burst(NonZeroU32::new(burst.max(1)).unwrap());
    RateLimiter::direct(quota)
}

#[derive(Debug)]
pub struct StellarRateLimiter {
    inner: DefaultRateLimiter,
}

impl StellarRateLimiter {
    pub fn new(per_second: u32, burst: u32) -> Self {
        Self {
            inner: build_rate_limiter(per_second, burst),
        }
    }

    pub fn try_acquire(&self) -> bool {
        self.inner.check().is_ok()
    }

    pub async fn acquire(&self) {
        loop {
            match self.inner.check() {
                Ok(()) => return,
                Err(negative) => {
                    let delay = negative.wait_time_from(QuantaClock::default().now());
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    pub fn wait_time(&self) -> Option<Duration> {
        self.inner
            .check()
            .err()
            .map(|negative| negative.wait_time_from(QuantaClock::default().now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_allows_burst_within_configured_limit() {
        let limiter = StellarRateLimiter::new(1, 2);

        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(!limiter.try_acquire());
    }

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
