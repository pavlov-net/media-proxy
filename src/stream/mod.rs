//! Stream orchestration: per-stream state machine, FrameSource enum,
//! producer/sampler (paced mode), native cadence runner.

pub mod fetcher;
pub mod frame_source;
pub mod handle;
pub mod http;
pub mod orchestrator;
pub mod probe;
pub mod runner;
pub mod state;
pub mod url;

pub use frame_source::FrameSource;
pub use handle::StreamHandle;
pub use orchestrator::Orchestrator;
pub use state::StreamState;
