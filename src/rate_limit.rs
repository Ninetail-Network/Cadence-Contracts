use governor::{clock::{Clock, MonotonicClock}, Quota, RateLimiter};
use std::{num::NonZeroU32, time::Duration};

pub type DefaultRateLimiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    MonotonicClock,
>;

pub fn build_rate_limiter(per_second: u32, burst: u32) -> DefaultRateLimiter {
    let quota = Quota::per_second(NonZeroU32::new(per_second).unwrap())
        .allow_burst(NonZeroU32::new(burst).unwrap());
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
                    let delay = negative.wait_time_from(MonotonicClock {}.now());
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    pub fn wait_time(&self) -> Option<Duration> {
        self.inner
            .check()
            .err()
            .map(|negative| negative.wait_time_from(MonotonicClock {}.now()))
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
}
