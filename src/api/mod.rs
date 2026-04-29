//! HTTP + WebSocket server.

pub mod animimg;
pub mod health;
pub mod internal;
pub mod state;

pub use state::AppState;

use axum::http::HeaderMap;

/// Extract the `Host` header for use as `server_host` in source
/// normalization. Falls back to `"localhost"` when missing or non-ASCII so
/// `internal:` rewriting still produces a valid URL.
pub fn host_header(headers: &HeaderMap) -> &str {
    headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost")
}

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use tokio::net::TcpListener;
use tracing::info;

use crate::resolver::{HttpResolver, NoopResolver, PassthroughLayer, Resolver, SubprocessResolver};
use crate::stream::Orchestrator;
use crate::yt_dlp::YtDlp;
use crate::{Config, Result};

pub async fn serve(addr: SocketAddr, config: Config) -> Result<()> {
    let config = Arc::new(config);
    let resolver: Arc<dyn Resolver> = build_resolver(&config)?;

    // Warm hwaccel probe off the request path (~25-100ms per backend).
    tokio::task::spawn_blocking(|| {
        let probed = crate::video::hwaccel::available();
        info!(?probed, "hwaccel probe complete");
    });

    let orch = Arc::new(Orchestrator::new(config.clone(), resolver));
    let state = Arc::new(AppState {
        config: config.clone(),
        orch,
    });

    let app = Router::new()
        .route("/api/system/health", get(health::health))
        .route("/api/convert/animimg", post(animimg::convert))
        .route("/api/internal/placeholder/{*spec}", get(internal::placeholder))
        .route(
            "/api/internal/homeassistant/{*spec}",
            get(internal::homeassistant),
        )
        .route("/control", get(crate::control::ws::upgrade))
        .with_state(state);

    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "server listening");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .map_err(|e| crate::error::Error::Other(e.to_string()))?;
    Ok(())
}

/// Explicit `resolver.url` wins; otherwise auto-detect local `yt-dlp`;
/// otherwise fail closed. Always wrapped in `PassthroughLayer` so direct
/// media short-circuits.
fn build_resolver(config: &Arc<Config>) -> Result<Arc<dyn Resolver>> {
    let timeout = Duration::from_millis(config.resolver.timeout_ms);
    let inner: Box<dyn Resolver> = if let Some(url) = &config.resolver.url {
        info!(%url, "resolver: http sidecar");
        Box::new(HttpResolver::new(url.clone(), timeout).map_err(crate::error::Error::Resolver)?)
    } else if let Some(yt_dlp) = YtDlp::detect() {
        info!(bin = %yt_dlp.bin().display(), "resolver: yt-dlp subprocess");
        Box::new(SubprocessResolver::new(yt_dlp, timeout))
    } else {
        info!("resolver: none (yt-dlp not on PATH and resolver.url unset; non-direct URLs will fail)");
        Box::new(NoopResolver)
    };
    Ok(Arc::new(PassthroughLayer::new(inner)))
}
