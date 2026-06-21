use prometheus::{
    Counter, Encoder, Gauge, HistogramOpts, HistogramVec, IntCounter, IntCounterVec,
    Opts, Registry, TextEncoder,
};
use std::prelude::v1::*;
use std::sync::Arc;
use std::time::Instant;

/// Central metrics registry wrapping Prometheus instrumentation.
///
/// This registry is shared across all service modules via `Arc<MetricsRegistry>`
/// and exposes a Prometheus text-format endpoint through `render()`.
pub struct MetricsRegistry {
    registry: Registry,

    // ── General request metrics ──
    request_count: Counter,
    error_count: Counter,

    // ── Cache metrics ──
    cache_hits: IntCounter,
    cache_misses: IntCounter,
    cache_expired: IntCounter,
    cache_serialization_failures: IntCounter,

    // ── Document registration metrics ──
    document_registration_total: IntCounterVec,
    document_revocation_total: IntCounterVec,

    // ── Verification metrics ──
    verification_total: IntCounterVec,
    verification_latency_seconds: HistogramVec,
    horizon_latency_seconds: HistogramVec,
    retry_total: IntCounter,

    // ── Rate limiter metrics ──
    rate_limit_tokens_consumed: IntCounter,
    rate_limit_violations: IntCounter,
    issuer_rate_limit_violations: IntCounter,
    address_rate_limit_violations: IntCounter,
    issuer_rate_limit_resets: IntCounter,
    address_rate_limit_resets: IntCounter,

    // ── Event ingestion metrics ──
    event_duplicates: IntCounter,
    event_ordering_failures: IntCounter,
    event_backlog_size: Gauge,

    // ── Config validation metrics ──
    config_validation_failures: IntCounter,
    config_reload_total: IntCounter,
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRegistry {
    pub fn new() -> Self {
        let registry = Registry::new();

        // ── General request metrics ──
        let request_count =
            Counter::new("requests_total", "Total number of API requests").unwrap();
        let error_count =
            Counter::new("errors_total", "Total number of errors encountered").unwrap();

        // ── Cache metrics ──
        let cache_hits = IntCounter::new("cache_hits_total", "Total cache hits").unwrap();
        let cache_misses = IntCounter::new("cache_misses_total", "Total cache misses").unwrap();
        let cache_expired = IntCounter::new(
            "cache_expired_total",
            "Total cache entries returned as miss due to TTL expiry",
        )
        .unwrap();
        let cache_serialization_failures = IntCounter::new(
            "cache_serialization_failures_total",
            "Total cache serialization/deserialization failures",
        )
        .unwrap();

        // ── Document metrics ──
        let document_registration_total = IntCounterVec::new(
            Opts::new(
                "document_registration_total",
                "Total document registrations by outcome",
            ),
            &["status"],
        )
        .unwrap();

        let document_revocation_total = IntCounterVec::new(
            Opts::new(
                "document_revocation_total",
                "Total document revocations by outcome",
            ),
            &["status"],
        )
        .unwrap();

        // ── Verification metrics ──
        let verification_total = IntCounterVec::new(
            Opts::new("verification_total", "Total verifications by outcome"),
            &["status"],
        )
        .unwrap();

        let verification_latency_seconds = HistogramVec::new(
            HistogramOpts::new(
                "verification_latency_seconds",
                "End-to-end verification latency in seconds",
            ),
            &["status"],
        )
        .unwrap();

        let horizon_latency_seconds = HistogramVec::new(
            HistogramOpts::new(
                "horizon_latency_seconds",
                "Stellar Horizon API call latency in seconds",
            ),
            &["status"],
        )
        .unwrap();

        let retry_total =
            IntCounter::new("retry_total", "Total number of retry attempts across all operations")
                .unwrap();

        // ── Rate limiter metrics ──
        let rate_limit_tokens_consumed = IntCounter::new(
            "rate_limit_tokens_consumed_total",
            "Total rate limiter tokens consumed",
        )
        .unwrap();

        let rate_limit_violations = IntCounter::new(
            "rate_limit_violations_total",
            "Total rate limit violations (requests rejected)",
        )
        .unwrap();

        let issuer_rate_limit_violations = IntCounter::new(
            "issuer_rate_limit_violations_total",
            "Total per-issuer rate limit violations",
        )
        .unwrap();

        let address_rate_limit_violations = IntCounter::new(
            "address_rate_limit_violations_total",
            "Total per-address rate limit violations",
        )
        .unwrap();

        let issuer_rate_limit_resets = IntCounter::new(
            "issuer_rate_limit_resets_total",
            "Total per-issuer rate limit bucket refills from exhaustion",
        )
        .unwrap();

        let address_rate_limit_resets = IntCounter::new(
            "address_rate_limit_resets_total",
            "Total per-address rate limit bucket refills from exhaustion",
        )
        .unwrap();

        // ── Event ingestion metrics ──
        let event_duplicates = IntCounter::new(
            "event_duplicates_total",
            "Total duplicate events detected and discarded",
        )
        .unwrap();

        let event_ordering_failures = IntCounter::new(
            "event_ordering_failures_total",
            "Total events rejected due to ordering/sequence failures",
        )
        .unwrap();

        let event_backlog_size = Gauge::new(
            "event_backlog_size",
            "Current number of unprocessed events in the backlog queue",
        )
        .unwrap();

        // ── Config validation metrics ──
        let config_validation_failures = IntCounter::new(
            "config_validation_failures_total",
            "Total configuration validation failures",
        )
        .unwrap();

        let config_reload_total = IntCounter::new(
            "config_reload_total",
            "Total configuration reloads attempted",
        )
        .unwrap();

        // Register everything
        registry
            .register(Box::new(request_count.clone()))
            .unwrap();
        registry
            .register(Box::new(error_count.clone()))
            .unwrap();
        registry
            .register(Box::new(cache_hits.clone()))
            .unwrap();
        registry
            .register(Box::new(cache_misses.clone()))
            .unwrap();
        registry
            .register(Box::new(cache_expired.clone()))
            .unwrap();
        registry
            .register(Box::new(cache_serialization_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(document_registration_total.clone()))
            .unwrap();
        registry
            .register(Box::new(document_revocation_total.clone()))
            .unwrap();
        registry
            .register(Box::new(verification_total.clone()))
            .unwrap();
        registry
            .register(Box::new(verification_latency_seconds.clone()))
            .unwrap();
        registry
            .register(Box::new(horizon_latency_seconds.clone()))
            .unwrap();
        registry
            .register(Box::new(retry_total.clone()))
            .unwrap();
        registry
            .register(Box::new(rate_limit_tokens_consumed.clone()))
            .unwrap();
        registry
            .register(Box::new(rate_limit_violations.clone()))
            .unwrap();
        registry
            .register(Box::new(issuer_rate_limit_violations.clone()))
            .unwrap();
        registry
            .register(Box::new(address_rate_limit_violations.clone()))
            .unwrap();
        registry
            .register(Box::new(issuer_rate_limit_resets.clone()))
            .unwrap();
        registry
            .register(Box::new(address_rate_limit_resets.clone()))
            .unwrap();
        registry
            .register(Box::new(event_duplicates.clone()))
            .unwrap();
        registry
            .register(Box::new(event_ordering_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(event_backlog_size.clone()))
            .unwrap();
        registry
            .register(Box::new(config_validation_failures.clone()))
            .unwrap();
        registry
            .register(Box::new(config_reload_total.clone()))
            .unwrap();

        Self {
            registry,
            request_count,
            error_count,
            cache_hits,
            cache_misses,
            cache_expired,
            cache_serialization_failures,
            document_registration_total,
            document_revocation_total,
            verification_total,
            verification_latency_seconds,
            horizon_latency_seconds,
            retry_total,
            rate_limit_tokens_consumed,
            rate_limit_violations,
            event_duplicates,
            event_ordering_failures,
            event_backlog_size,
            config_validation_failures,
            config_reload_total,
        }
    }

    /// Return a sharable `Arc<MetricsRegistry>` for use across service threads.
    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    // ── Request metrics ──────────────────────────────────────────────────

    pub fn increment_request_count(&self) {
        self.request_count.inc();
    }

    pub fn increment_error_count(&self) {
        self.error_count.inc();
    }

    // ── Cache metrics ────────────────────────────────────────────────────

    pub fn increment_cache_hits(&self) {
        self.cache_hits.inc();
    }

    pub fn increment_cache_misses(&self) {
        self.cache_misses.inc();
    }

    pub fn increment_cache_expired(&self) {
        self.cache_expired.inc();
    }

    pub fn increment_cache_serialization_failures(&self) {
        self.cache_serialization_failures.inc();
    }

    // ── Document metrics ─────────────────────────────────────────────────

    pub fn record_document_registration(&self, status: &str) {
        self.document_registration_total
            .with_label_values(&[status])
            .inc();
    }

    pub fn record_document_revocation(&self, status: &str) {
        self.document_revocation_total
            .with_label_values(&[status])
            .inc();
    }

    // ── Verification metrics ─────────────────────────────────────────────

    pub fn record_verification(&self, status: &str, latency_secs: f64) {
        self.verification_total.with_label_values(&[status]).inc();
        self.verification_latency_seconds
            .with_label_values(&[status])
            .observe(latency_secs);
    }

    pub fn record_horizon_latency(&self, status: &str, latency_secs: f64) {
        self.horizon_latency_seconds
            .with_label_values(&[status])
            .observe(latency_secs);
    }

    pub fn increment_retry(&self) {
        self.retry_total.inc();
    }

    // ── Rate limiter metrics ─────────────────────────────────────────────

    pub fn record_token_consumed(&self) {
        self.rate_limit_tokens_consumed.inc();
    }

    pub fn increment_rate_limit_violation(&self) {
        self.rate_limit_violations.inc();
    }

    // ── Event ingestion metrics ──────────────────────────────────────────

    pub fn increment_event_duplicate(&self) {
        self.event_duplicates.inc();
    }

    pub fn increment_event_ordering_failure(&self) {
        self.event_ordering_failures.inc();
    }

    pub fn set_event_backlog(&self, size: i64) {
        self.event_backlog_size.set(size as f64);
    }

    pub fn increment_event_backlog(&self) {
        self.event_backlog_size.inc();
    }

    pub fn decrement_event_backlog(&self) {
        self.event_backlog_size.dec();
    }

    // ── Config metrics ───────────────────────────────────────────────────

    pub fn increment_config_validation_failure(&self) {
        self.config_validation_failures.inc();
    }

    pub fn increment_config_reload(&self) {
        self.config_reload_total.inc();
    }

    // ── Latency helper ───────────────────────────────────────────────────

    /// Start a timer for measuring operation latency.
    pub fn start_timer() -> Instant {
        Instant::now()
    }

    /// Return elapsed seconds since `start`.
    pub fn elapsed_secs(start: Instant) -> f64 {
        start.elapsed().as_secs_f64()
    }

    // ── Prometheus rendering ─────────────────────────────────────────────

    /// Render all registered metrics in Prometheus text format.
    ///
    /// Returns a `String` suitable for direct HTTP response or Prometheus scraping.
    /// Callers can wrap this with `axum::response::IntoResponse` at the HTTP layer.
    pub fn render(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder
            .encode(&metric_families, &mut buffer)
            .unwrap_or_default();
        String::from_utf8(buffer).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_creates_all_metrics() {
        let metrics = MetricsRegistry::new();

        // Smoke test: invoke each counter at least once
        metrics.increment_request_count();
        metrics.increment_error_count();
        metrics.increment_cache_hits();
        metrics.increment_cache_misses();
        metrics.increment_cache_expired();
        metrics.increment_cache_serialization_failures();
        metrics.record_document_registration("success");
        metrics.record_document_registration("error");
        metrics.record_document_revocation("success");
        metrics.record_verification("success", 0.1);
        metrics.record_verification("failure", 0.2);
        metrics.record_horizon_latency("success", 0.05);
        metrics.record_horizon_latency("error", 1.0);
        metrics.increment_retry();
        metrics.record_token_consumed();
        metrics.increment_rate_limit_violation();
        metrics.increment_event_duplicate();
        metrics.increment_event_ordering_failure();
        metrics.set_event_backlog(5);
        metrics.increment_event_backlog();
        metrics.decrement_event_backlog();
        metrics.increment_config_validation_failure();
        metrics.increment_config_reload();

        let output = metrics.render();
        // Verify key metric names appear in the rendered output
        assert!(output.contains("requests_total"));
        assert!(output.contains("cache_hits_total"));
        assert!(output.contains("verification_total"));
        assert!(output.contains("horizon_latency_seconds"));
        assert!(output.contains("rate_limit_violations_total"));
        assert!(output.contains("event_backlog_size"));
        assert!(output.contains("config_validation_failures_total"));
    }

    #[test]
    fn timer_returns_positive_elapsed() {
        let start = MetricsRegistry::start_timer();
        let elapsed = MetricsRegistry::elapsed_secs(start);
        assert!(elapsed >= 0.0);
    }

    #[test]
    fn arc_creates_shared_registry() {
        let metrics = MetricsRegistry::arc();
        metrics.increment_request_count();
        let output = metrics.render();
        assert!(output.contains("requests_total"));
    }
}
