//! Layered error taxonomy. Each subsystem owns its own `thiserror` enum,
//! converted into this top-level `Error` via `#[from]`.

use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(#[from] ConfigError),

    #[error("video: {0}")]
    Video(#[from] VideoError),

    #[error("image: {0}")]
    Image(#[from] ImageError),

    #[error("output: {0}")]
    Output(#[from] OutputError),

    #[error("resolver: {0}")]
    Resolver(#[from] ResolverError),

    #[error("stream: {0}")]
    Stream(#[from] StreamError),

    #[error("control: {0}")]
    Control(#[from] ControlError),

    #[error("render: {0}")]
    Render(#[from] RenderError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("addr parse: {0}")]
    AddrParse(#[from] std::net::AddrParseError),

    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("figment: {0}")]
    Figment(#[from] Box<figment::Error>),

    #[error("{0}")]
    Invalid(String),
}

impl From<figment::Error> for ConfigError {
    fn from(e: figment::Error) -> Self {
        Self::Figment(Box::new(e))
    }
}

/// Media-layer error taxonomy, library-agnostic.
///
/// `retryable` indicates whether the caller's retry loop should attempt again.
/// `source_url` carries the input URL so logs/errors can identify the stream.
#[derive(Debug, Error)]
pub enum MediaError {
    #[error("network error ({source_url}): {message}")]
    Network {
        source_url: String,
        message: String,
        error_code: Option<i32>,
        retryable: bool,
    },

    #[error("format error ({source_url}): {message}")]
    Format { source_url: String, message: String },

    #[error("decode error ({source_url}): {message}")]
    Decode {
        source_url: String,
        message: String,
        retryable: bool,
    },

    #[error("not found: {source_url}")]
    NotFound { source_url: String },

    #[error("media error ({source_url}): {message}")]
    Other { source_url: String, message: String },
}

impl MediaError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Network { retryable: true, .. } | Self::Decode { retryable: true, .. }
        )
    }

    pub fn source_url(&self) -> &str {
        match self {
            Self::Network { source_url, .. }
            | Self::Format { source_url, .. }
            | Self::Decode { source_url, .. }
            | Self::NotFound { source_url }
            | Self::Other { source_url, .. } => source_url,
        }
    }
}

#[derive(Debug, Error)]
pub enum VideoError {
    #[error(transparent)]
    Media(#[from] MediaError),

    #[error("ffmpeg: {0}")]
    Ffmpeg(String),

    #[error("filter graph: {0}")]
    FilterGraph(String),
}

#[derive(Debug, Error)]
pub enum ImageError {
    #[error(transparent)]
    Media(#[from] MediaError),

    #[error("decode: {0}")]
    Decode(String),

    #[error("resize: {0}")]
    Resize(String),

    #[error("bomb: image exceeds size limit ({actual} > {limit} bytes)")]
    DecompressionBomb { actual: usize, limit: usize },

    #[error("icc: {0}")]
    Icc(String),
}

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("address already in use: {0}")]
    AddressInUse(String),

    #[error("sink: {0}")]
    Sink(String),
}

#[derive(Debug, Error)]
pub enum ResolverError {
    #[error("http: {0}")]
    Http(String),

    #[error("invalid response: {0}")]
    InvalidResponse(String),

    #[error("unavailable: {0}")]
    Unavailable(String),
}

impl From<reqwest::Error> for ResolverError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e.to_string())
    }
}

#[derive(Debug, Error)]
pub enum StreamError {
    #[error(transparent)]
    Media(#[from] MediaError),

    #[error(transparent)]
    Video(#[from] VideoError),

    #[error(transparent)]
    Image(#[from] ImageError),

    #[error(transparent)]
    Output(#[from] OutputError),

    #[error(transparent)]
    Resolver(#[from] ResolverError),

    #[error("invalid transition: {from} -> {to}")]
    InvalidTransition { from: &'static str, to: &'static str },

    #[error("stream cancelled")]
    Cancelled,
}

#[derive(Debug, Error)]
pub enum ControlError {
    #[error("protocol: {0}")]
    Protocol(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unknown output: {0}")]
    UnknownOutput(i32),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

impl ControlError {
    /// Wire-level error code used in `{"type":"error","code":...}` messages.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Protocol(_) => "proto",
            Self::BadRequest(_) => "bad_request",
            Self::UnknownOutput(_) => "bad_request",
            Self::Json(_) => "bad_request",
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum RenderError {
    #[error("font: {0}")]
    Font(String),

    #[error("parse: {0}")]
    Parse(String),
}
