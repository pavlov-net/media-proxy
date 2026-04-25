// Pedantic / nursery stay opt-in per-module (see rust.md §Pre-commit).
// The CI gate uses `-D warnings` so those groups must not fire by default.
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

pub use config::Config;
pub use error::{Error, Result};
