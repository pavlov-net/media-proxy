//! Frame source enum — the hot-path dispatch point for every stream.
//!
//! Enum (not trait object) per `rust.md` §Architectural Decisions:
//! pattern-match on one of `Video`, `StaticImage`, `Animated`.

use bytes::Bytes;

#[derive(Debug, Clone)]
pub struct RgbFrame {
    pub rgb888: Bytes,
    pub delay_ms: f32,
}

pub enum FrameSource {
    Video(VideoSource),
    StaticImage(StaticImageSource),
    Animated(AnimatedSource),
}

pub struct VideoSource {
    pub rx: tokio::sync::mpsc::Receiver<RgbFrame>,
}

pub struct StaticImageSource {
    pub frame: Bytes,
    pub emitted: bool,
}

pub struct AnimatedSource {
    pub frames: std::sync::Arc<crate::image::animated::cache::CachedSequence>,
    pub cursor: usize,
}

impl FrameSource {
    pub async fn next(&mut self) -> Option<RgbFrame> {
        match self {
            Self::Video(v) => v.rx.recv().await,
            Self::StaticImage(s) => {
                if s.emitted {
                    return None;
                }
                s.emitted = true;
                Some(RgbFrame {
                    rgb888: s.frame.clone(),
                    delay_ms: 100.0,
                })
            }
            Self::Animated(a) => {
                if a.cursor >= a.frames.frames.len() {
                    return None;
                }
                let (rgb888, delay_ms) = a.frames.frames[a.cursor].clone();
                a.cursor += 1;
                Some(RgbFrame { rgb888, delay_ms })
            }
        }
    }

    /// Reset iteration state so the next `next()` re-emits from the start.
    /// Returns `false` for `Video` — ffmpeg owns its own loop via
    /// `-stream_loop -1`, the orchestrator handles the rebuild path.
    pub fn try_rewind(&mut self) -> bool {
        match self {
            Self::Video(_) => false,
            Self::StaticImage(s) => {
                s.emitted = false;
                true
            }
            Self::Animated(a) => {
                a.cursor = 0;
                true
            }
        }
    }
}
