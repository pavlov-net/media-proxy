//! Frame sinks and metrics. DDP address reservations enforce stream exclusivity.

pub mod ddp;
pub mod metrics;
pub mod sink;

pub use sink::{Frame, FrameMeta, OutputSink, PixelFormat, StreamId};
