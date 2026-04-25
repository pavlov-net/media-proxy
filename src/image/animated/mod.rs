//! Animated image compositor.
//!
//! Three formats live here: GIF (via `gif` + `gif-dispose`), APNG (via `png`
//! with a manual disposal/blend compositor), and animated WebP (via
//! `image-webp` on git `main` for the post-#179 fixes).
//!
//! Each format decoder produces a stream of `AnimatedFrame` — fully
//! composited RGBA + per-frame duration — that the dispatcher runs through
//! the still-image pipeline and caches as RGB888.

use bytes::Bytes;

pub mod apng;
pub mod cache;
pub mod dispatch;
pub mod gif;
pub mod webp;

/// One fully-composited animation frame.
#[derive(Debug, Clone)]
pub struct AnimatedFrame {
    /// RGBA canvas at the animation's natural size.
    pub rgba: Bytes,
    pub width: u32,
    pub height: u32,
    /// Display duration for this frame.
    pub delay_ms: f32,
}

/// Minimum delay the output pipeline allows.
pub const MIN_DELAY_MS: f32 = 10.0;

/// Default when the source reports zero/unknown duration.
pub const DEFAULT_DELAY_MS: f32 = 100.0;

/// Concrete animated-decoder enum — hot-path dispatch per `rust.md` §3.
///
/// Each variant's inner state is heap-allocated via `Box`: the discriminant
/// lives on the stack, but the backing canvas buffers (~megabytes for
/// anything wider than a postage stamp) stay on the heap.
pub enum AnimatedDecoder {
    Gif(Box<gif::GifDecoder>),
    Apng(Box<apng::ApngDecoder>),
    Webp(Box<webp::WebpDecoder>),
}

impl AnimatedDecoder {
    pub fn next_frame(&mut self) -> Result<Option<AnimatedFrame>, crate::error::ImageError> {
        match self {
            Self::Gif(d) => d.next_frame(),
            Self::Apng(d) => d.next_frame(),
            Self::Webp(d) => d.next_frame(),
        }
    }
}
