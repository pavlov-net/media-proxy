//! Static + animated image pipeline.

pub mod animated;
pub mod decode;
pub mod dispatch;
pub mod gamma;
pub mod icc;
pub mod palette;
pub mod pipeline;
pub mod resize;
pub mod unsharp;

pub use pipeline::{ImagePipeline, PipelineParams};
