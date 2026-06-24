use soroban_sdk::{Address, Env};
use std::prelude::v1::*;

use crate::metrics::MetricsRegistry;
use std::sync::Arc;

/// Per-issuer rate limit state stored in the contract ledger.
///
/// Tracks token consumption for rate limiting operations per issuer address.
/// Tokens refill over time at the configured rate (tokens per second).
#[derive(Clone, Debug)]
pub struct IssuerRateLimitState {
    /// Available tokens for this issuer
    pub available_tokens: u32,
    /// Last time tokens were refilled (ledger timestamp)
    pub last_refill_timestamp: u64,
}

impl IssuerRateLimitState {
    /// Create a new rate limit state with full burst allowance.
    pub fn new(burst_allowance: u32, current_timestamp: u64) -> Self {
        Self {
            available_tokens: burst_allowance,
            last_refill_timestamp: current_timestamp,
        }
    }

    /// Calculate refilled tokens based on elapsed time and refill rate.
    ///
    /// Returns the new available token count (capped at max_tokens).
    pub fn refill(&mut self, per_second: u32, burst: u32, current_timestamp: u64) -> u32 {
        if current_timestamp > self.last_refill_timestamp {
            let elapsed_seconds = (current_timestamp - self.last_refill_timestamp) as u32;
            let tokens_to_add = elapsed_seconds.saturating_mul(per_second);
            self.available_tokens = self
                .available_tokens
                .saturating_add(tokens_to_add)
                .min(burst);
            self.last_refill_timestamp = current_timestamp;
        }
        self.available_tokens
    }

    /// Try to consume a token. Returns true if successful, false if rate limit exceeded.
    pub fn try_consume(&mut self, tokens_needed: u32) -> bool {
        if self.available_tokens >= tokens_needed {
            self.available_tokens -= tokens_needed;
            true
        } else {
            false
        }
    }
}

/// Manages per-issuer rate limiting for contract operations.
///
/// Uses the contract's persistent storage to maintain token counts per issuer.
/// Integrates with metrics for observability.
pub struct IssuerRateLimiter {
    per_second: u32,
    burst: u32,
    metrics: Option<Arc<MetricsRegistry>>,
}

impl IssuerRateLimiter {
    /// Create a new rate limiter with the specified per-second and burst settings.
    pub fn new(per_second: u32, burst: u32, metrics: Option<Arc<MetricsRegistry>>) -> Self {
        Self {
            per_second: per_second.max(1),
            burst: burst.max(1),
            metrics,
        }
    }

    /// Check if an issuer can consume tokens. Records metrics and returns success/failure.
    ///
    /// This is a non-destructive check—it does not update state. Use after this
    /// if you want to actually consume tokens and update persistent storage.
    pub fn check(
        &self,
        issuer_address: &str,
        tokens_needed: u32,
        available_tokens: u32,
    ) -> bool {
        if available_tokens >= tokens_needed {
            if let Some(ref m) = self.metrics {
                m.record_token_consumed();
            }
            true
        } else {
            if let Some(ref m) = self.metrics {
                m.increment_rate_limit_violation();
                m.record_rate_limit_hit(issuer_address);
            }
            false
        }
    }

    /// Calculate refilled tokens for an issuer based on elapsed time.
    ///
    /// Does not modify state; use this to determine current available tokens
    /// for a rate limit check.
    pub fn calculate_refilled_tokens(
        &self,
        current_available: u32,
        last_refill_timestamp: u64,
        current_timestamp: u64,
    ) -> u32 {
        if current_timestamp > last_refill_timestamp {
            let elapsed_seconds = (current_timestamp - last_refill_timestamp) as u32;
            let tokens_to_add = elapsed_seconds.saturating_mul(self.per_second);
            current_available
                .saturating_add(tokens_to_add)
                .min(self.burst)
        } else {
            current_available
        }
    }

    /// Update metrics when a rate limit reset occurs (tokens refilled).
    pub fn record_reset(&self, issuer_address: &str) {
        if let Some(ref m) = self.metrics {
            m.record_rate_limit_reset(issuer_address);
        }
    }

    pub fn per_second(&self) -> u32 {
        self.per_second
    }

    pub fn burst(&self) -> u32 {
        self.burst
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_state_new_sets_burst_allowance() {
        let state = IssuerRateLimitState::new(10, 1000);
        assert_eq!(state.available_tokens, 10);
        assert_eq!(state.last_refill_timestamp, 1000);
    }

    #[test]
    fn rate_limit_state_refill_adds_tokens() {
        let mut state = IssuerRateLimitState::new(5, 1000);
        // 10 seconds later, at 2 tokens/sec, should have 5 + 20 = 25 tokens
        // but capped at burst of 30, so 25
        let refilled = state.refill(2, 30, 1010);
        assert_eq!(refilled, 25);
        assert_eq!(state.available_tokens, 25);
    }

    #[test]
    fn rate_limit_state_refill_caps_at_burst() {
        let mut state = IssuerRateLimitState::new(5, 1000);
        // 100 seconds later would give many tokens, but capped at burst
        let refilled = state.refill(10, 20, 1100);
        assert_eq!(refilled, 20); // Capped at burst
    }

    #[test]
    fn rate_limit_state_try_consume_success() {
        let mut state = IssuerRateLimitState::new(10, 1000);
        assert!(state.try_consume(5));
        assert_eq!(state.available_tokens, 5);
    }

    #[test]
    fn rate_limit_state_try_consume_failure() {
        let mut state = IssuerRateLimitState::new(5, 1000);
        assert!(!state.try_consume(10)); // Need 10, only have 5
        assert_eq!(state.available_tokens, 5); // Token count unchanged
    }

    #[test]
    fn issuer_rate_limiter_check_success() {
        let limiter = IssuerRateLimiter::new(10, 20, None);
        assert!(limiter.check("issuer_1", 5, 10));
    }

    #[test]
    fn issuer_rate_limiter_check_failure() {
        let limiter = IssuerRateLimiter::new(10, 20, None);
        assert!(!limiter.check("issuer_1", 15, 10));
    }

    #[test]
    fn issuer_rate_limiter_calculate_refilled_tokens() {
        let limiter = IssuerRateLimiter::new(10, 100, None);
        // 5 seconds elapsed, 10 per second = 50 tokens
        let refilled = limiter.calculate_refilled_tokens(30, 1000, 1005);
        assert_eq!(refilled, 80); // 30 + 50 = 80
    }

    #[test]
    fn issuer_rate_limiter_calculate_refilled_tokens_respects_burst() {
        let limiter = IssuerRateLimiter::new(10, 50, None);
        // Would refill to 80, but capped at burst of 50
        let refilled = limiter.calculate_refilled_tokens(30, 1000, 1010);
        assert_eq!(refilled, 50);
    }
}
