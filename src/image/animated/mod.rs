//! GIF, APNG and WebP decoders return composited RGBA frames and durations.
//! The dispatcher resizes them through the image pipeline and caches RGB888.

pub mod apng;
pub mod cache;
pub mod dispatch;
pub mod gif;
pub mod webp;

/// Composited animation frame owned by the image pipeline to avoid an extra copy.
#[derive(Debug, Clone)]
pub struct AnimatedFrame {
    /// RGBA canvas at the animation's natural size.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Display duration in milliseconds.
    pub delay_ms: f32,
}

/// Minimum delay the output pipeline allows.
pub const MIN_DELAY_MS: f32 = 10.0;

/// Default when the source reports zero/unknown duration.
pub const DEFAULT_DELAY_MS: f32 = 100.0;

/// Format-specific decoders with heap-allocated decoder state.
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
