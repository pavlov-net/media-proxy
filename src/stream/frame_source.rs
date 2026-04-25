//! Frame source enum — the hot-path dispatch point for every stream.
//!
//! Enum (not trait object) per `rust.md` §Architectural Decisions:
//! pattern-match on one of `Video`, `StaticImage`, `Animated`.

use bytes::Bytes;

/// A frame emitted by a `FrameSource`: RGB888 bytes + the post-processing
/// delay hint the producer wants the consumer to respect.
#[derive(Debug, Clone)]
pub struct RgbFrame {
    pub rgb888: Bytes,
    pub delay_ms: f32,
}

/// Enum over the three concrete source types.
pub enum FrameSource {
    Video(VideoSource),
    StaticImage(StaticImageSource),
    Animated(AnimatedSource),
}

pub struct VideoSource {
    pub rx: tokio::sync::mpsc::Receiver<RgbFrame>,
}

pub struct StaticImageSource {
    /// Pre-rendered RGB888 for the target size. Re-emitted forever (for
    /// looping stills) or once (for single-shot).
    pub frame: Bytes,
    pub r#loop: bool,
    pub emitted: bool,
}

pub struct AnimatedSource {
    /// Pre-rendered RGB888 frames at the target size + their delays.
    pub frames: std::sync::Arc<crate::image::animated::cache::CachedSequence>,
    pub r#loop: bool,
    pub cursor: usize,
    pub loops_done: u32,
}

impl FrameSource {
    pub async fn next(&mut self) -> Option<RgbFrame> {
        match self {
            Self::Video(v) => v.rx.recv().await,
            Self::StaticImage(s) => {
                if !s.r#loop && s.emitted {
                    return None;
                }
                s.emitted = true;
                Some(RgbFrame {
                    rgb888: s.frame.clone(),
                    delay_ms: 100.0,
                })
            }
            Self::Animated(a) => {
                if a.frames.frames.is_empty() {
                    return None;
                }
                if a.cursor >= a.frames.frames.len() {
                    if !a.r#loop {
                        return None;
                    }
                    a.cursor = 0;
                    a.loops_done += 1;
                }
                let (rgb888, delay_ms) = a.frames.frames[a.cursor].clone();
                a.cursor += 1;
                Some(RgbFrame { rgb888, delay_ms })
            }
        }
    }
}
