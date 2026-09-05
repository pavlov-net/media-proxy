//! Media decoding and WebSocket-controlled DDP streaming.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod api;
pub mod config;
pub mod control;
pub mod error;
pub mod image;
pub mod output;
pub mod platform;
pub mod render;
pub mod resolver;
pub mod stream;
pub mod telemetry;
pub mod video;
pub mod yt_dlp;

pub use config::Config;
pub use error::{Error, Result};
