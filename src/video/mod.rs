//! Video decoding through ffmpeg. [`dispatch`] builds sources for the stream runners.

pub mod autocrop;
pub mod dispatch;
pub mod filter_graph;
pub mod hwaccel;
pub mod probe;
pub mod subprocess;
pub mod timing;

pub use filter_graph::{FilterGraphParams, build_filter_graph};
