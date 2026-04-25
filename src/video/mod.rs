//! Video pipeline — ffmpeg subprocess via `tokio::process`.
//!
//! Split into:
//! - `filter_graph` — pure function, build the `-vf` chain (table-driven tests).
//! - `subprocess`   — spawn ffmpeg, pipe RGB24 frames, parse `-vf showinfo` stderr for PTS.
//! - `timing`       — consecutive-PTS → per-frame display delay.
//! - `dispatch`     — resolver → ffmpeg spawn → `VideoSource` channel.
//! - `autocrop`     — short probe for black-bar detection.
//! - `hwaccel`      — map our `HwBackend` enum to ffmpeg CLI flags.

pub mod autocrop;
pub mod dispatch;
pub mod filter_graph;
pub mod hwaccel;
pub mod subprocess;
pub mod timing;

pub use filter_graph::{FilterGraphParams, build_filter_graph};
