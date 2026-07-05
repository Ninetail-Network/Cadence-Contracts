//! Cadence service binary entry point.
//!
//! This binary starts an HTTP server that exposes:
//!
//! - `GET /health`  — JSON health check
//! - `GET /metrics` — Prometheus text-format metrics (via [`MetricsRegistry`])
//!
//! # Configuration
//!
//! All settings are read from environment variables. See [`AppConfig::from_env`]
//! for the full reference.
//!
//! # Running
//!
//! ```bash
//! export STELLAR_SECRET_KEY="SBU2R..."
//! cargo run --release
//! ```
//!
//! The server binds to `0.0.0.0:{PORT}` (default `8080`).

// ── WASM stub ────────────────────────────────────────────────────────────
// The binary only works on native targets.  Provide a stub so that `cargo
// build --target wasm32-unknown-unknown` does not error on the bin target.

#[cfg(target_arch = "wasm32")]
fn main() {
    eprintln!("error: this service binary does not run under wasm32");
    std::process::exit(1);
}

// ── Native server entry point ────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use axum::extract::State;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;

    use Cadence_contract::config::AppConfig;
    use Cadence_contract::metrics::MetricsRegistry;
    use Cadence_contract::webhook::WebhookDispatcher;

    /// Shared application state, accessible by all axum handlers.
    #[derive(Clone)]
    struct AppState {
        metrics: Arc<MetricsRegistry>,
        webhook: Arc<WebhookDispatcher>,
    }

    /// Build the axum router with all application routes.
    fn build_router(state: AppState) -> Router {
        Router::new()
            .route("/health", get(health_handler))
            .route("/metrics", get(metrics_handler))
            .route("/webhooks/dlq", get(dlq_status_handler))
            .route("/webhooks/dlq/drain", post(dlq_drain_handler))
            .with_state(state)
    }

    /// `GET /health` — returns a JSON health-check payload.
    async fn health_handler() -> impl IntoResponse {
        Json(json!({"status": "ok"}))
    }

    /// `GET /metrics` — returns Prometheus text-format metrics.
    async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
        state.metrics.render()
    }

    /// `GET /webhooks/dlq` — returns the current DLQ depth.
    async fn dlq_status_handler(State(state): State<AppState>) -> impl IntoResponse {
        let depth = state.webhook.dlq_depth().await;
        Json(json!({ "dlq_depth": depth }))
    }

    /// `POST /webhooks/dlq/drain` — drains and returns all DLQ entries for manual replay.
    async fn dlq_drain_handler(State(state): State<AppState>) -> impl IntoResponse {
        let entries = state.webhook.drain_dlq().await;
        Json(json!({ "drained": entries.len(), "entries": entries }))
    }

    /// Bootstrap: load config, wire up services, and start the server.
    pub async fn run() -> anyhow::Result<()> {
        // ── Metrics ─────────────────────────────────────────────────
        let metrics = MetricsRegistry::arc();

        // ── Configuration ───────────────────────────────────────────
        let config = AppConfig::from_env_with_metrics(Some(Arc::clone(&metrics)))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        eprintln!("[Cadence] Configuration loaded successfully");
        eprintln!("[Cadence]   port:               {}", config.port);
        eprintln!(
            "[Cadence]   stellar_horizon_url: {}",
            config.stellar_horizon_url
        );
        eprintln!("[Cadence]   redis_url:           {}", config.redis_url);
        eprintln!(
            "[Cadence]   rate_limit:          {}/s (burst {})",
            config.rate_limit_per_second, config.rate_limit_burst
        );
        eprintln!(
            "[Cadence]   webhooks:            {} url(s) configured (max_retries={})",
            config.webhook_urls.len(),
            config.webhook_max_retries,
        );

        // ── Webhook dispatcher ───────────────────────────────────────
        let webhook = Arc::new(WebhookDispatcher::from_app_config(
            &config,
            Some(Arc::clone(&metrics)),
        ));

        // ── Router ──────────────────────────────────────────────────
        let state = AppState {
            metrics: Arc::clone(&metrics),
            webhook,
        };
        let app = build_router(state);

        // ── Bind & serve ────────────────────────────────────────────
        let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
        eprintln!("[Cadence] Starting HTTP server on {addr}");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
    if let Err(e) = native::run().await {
        eprintln!("[Cadence] Fatal error: {e:#}");
        std::process::exit(1);
    }
}
