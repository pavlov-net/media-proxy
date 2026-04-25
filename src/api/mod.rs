//! HTTP + WebSocket server.

pub mod animimg;
pub mod health;
pub mod internal;
pub mod state;

pub use state::AppState;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use tokio::net::TcpListener;
use tracing::info;

use crate::resolver::{HttpResolver, Resolver};
use crate::stream::Orchestrator;
use crate::{Config, Result};

pub async fn serve(addr: SocketAddr, config: Config) -> Result<()> {
    let config = Arc::new(config);
    let resolver: Arc<dyn Resolver> = build_resolver(&config)?;

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

fn build_resolver(config: &Arc<Config>) -> Result<Arc<dyn Resolver>> {
    match &config.resolver.url {
        Some(url) => {
            let r = HttpResolver::new(url.clone(), Duration::from_millis(config.resolver.timeout_ms))
                .map_err(crate::error::Error::Resolver)?;
            Ok(Arc::new(r) as Arc<dyn Resolver>)
        }
        None => {
            // No resolver configured — use a passthrough fake (only safe for
            // direct URLs; anything needing resolution will error).
            Ok(Arc::new(crate::resolver::FakeResolver::new().with_passthrough()) as Arc<dyn Resolver>)
        }
    }
}
