//! Output sinks. DDP is the only sink at present; new protocols slot in
//! alongside it.
//!
//! Conflict addressing is a *sink* concern: each sink implementation owns
//! its own collision rules via `reserve`.

pub mod ddp;
pub mod metrics;
pub mod sink;

pub use sink::{Frame, FrameMeta, OutputSink, PixelFormat, StreamId};
