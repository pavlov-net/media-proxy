use std::net::{IpAddr, SocketAddr};

use clap::Parser;
use media_proxy::{Config, Result, api, telemetry};
use tracing::{info, warn};

#[derive(Debug, Parser)]
#[command(name = "media-proxy", version, about = "Media proxy → DDP for LED displays")]
struct Cli {
    #[arg(long, default_value = "0.0.0.0")]
    host: IpAddr,

    #[arg(long, default_value_t = 8788)]
    port: u16,

    #[arg(long)]
    config: Option<std::path::PathBuf>,

    #[arg(long, value_enum)]
    log_level: Option<telemetry::LogLevel>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load(cli.config.as_deref())?;
    telemetry::init(cli.log_level.unwrap_or(config.log.level));

    info!(version = env!("CARGO_PKG_VERSION"), "media-proxy starting");

    #[cfg(windows)]
    let _timer = media_proxy::platform::WindowsTimerResolution::new(config.net.win_timer_res);

    let addr = SocketAddr::new(cli.host, cli.port);

    tokio::select! {
        r = api::serve(addr, config) => r?,
        () = shutdown_signal() => {
            info!("shutdown signal received, draining streams");
        }
    }
    Ok(())
}

/// Resolve when the process receives SIGINT (Ctrl-C) or, on Unix, SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!(error = %e, "ctrl_c handler failed");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let sigterm = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                warn!(error = %e, "SIGTERM handler failed");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let sigterm = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = sigterm => {},
    }
}
