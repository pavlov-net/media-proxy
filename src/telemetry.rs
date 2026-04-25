use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Warning,
    Error,
    Critical,
}

impl LogLevel {
    pub fn as_filter(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warn",
            Self::Error | Self::Critical => "error",
        }
    }
}

/// Initialize `tracing` with a compact output format that's readable at a
/// glance and cheap to grep: `LEVEL target: message field=value …`. Drops
/// the default ISO-8601 timestamp (systemd / Docker / the tty already stamp
/// lines) and the ANSI colors (use `RUST_LOG_STYLE=always` to force them).
pub fn init(level: LogLevel) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("media_proxy={}", level.as_filter())));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .without_time()
        .with_target(true)
        .with_level(true)
        .with_ansi(
            std::env::var_os("RUST_LOG_STYLE")
                .map(|v| v == "always")
                .unwrap_or(false),
        )
        .compact()
        .try_init();
}
