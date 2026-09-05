use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[serde(alias = "DEBUG")]
    Debug,
    #[default]
    #[serde(alias = "INFO")]
    Info,
    #[serde(alias = "WARNING", alias = "warn", alias = "WARN")]
    Warning,
    #[serde(alias = "ERROR")]
    Error,
    #[serde(alias = "CRITICAL")]
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

/// Initializes compact tracing without timestamps. `RUST_LOG` overrides the
/// configured level; `RUST_LOG_STYLE=always` enables ANSI colors.
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
