//! ProofStell service binary entry point.
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
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;

    use proofstell_contract::config::AppConfig;
    use proofstell_contract::metrics::MetricsRegistry;

    /// Shared application state, accessible by all axum handlers.
    #[derive(Clone)]
    struct AppState {
        metrics: Arc<MetricsRegistry>,
    }

    /// Build the axum router with all application routes.
    fn build_router(state: AppState) -> Router {
        Router::new()
            .route("/health", get(health_handler))
            .route("/metrics", get(metrics_handler))
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

    /// Bootstrap: load config, wire up services, and start the server.
    pub async fn run() -> anyhow::Result<()> {
        // ── Metrics ─────────────────────────────────────────────────
        let metrics = MetricsRegistry::arc();

        // ── Configuration ───────────────────────────────────────────
        let config = AppConfig::from_env_with_metrics(Some(Arc::clone(&metrics)))
            .map_err(|e| anyhow::anyhow!("{e}"))?;

        eprintln!("[proofstell] Configuration loaded successfully");
        eprintln!("[proofstell]   port:               {}", config.port);
        eprintln!(
            "[proofstell]   stellar_horizon_url: {}",
            config.stellar_horizon_url
        );
        eprintln!("[proofstell]   redis_url:           {}", config.redis_url);
        eprintln!(
            "[proofstell]   rate_limit:          {}/s (burst {})",
            config.rate_limit_per_second, config.rate_limit_burst
        );

        // ── Router ──────────────────────────────────────────────────
        let state = AppState {
            metrics: Arc::clone(&metrics),
        };
        let app = build_router(state);

        // ── Bind & serve ────────────────────────────────────────────
        let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
        eprintln!("[proofstell] Starting HTTP server on {addr}");

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;

        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() {
    if let Err(e) = native::run().await {
        eprintln!("[proofstell] Fatal error: {e:#}");
        std::process::exit(1);
    }
}
