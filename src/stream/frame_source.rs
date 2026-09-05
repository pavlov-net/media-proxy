//! Frame iteration for video channels, static images, and cached animations.

use bytes::Bytes;
use tokio::sync::oneshot;

use crate::error::{MediaError, StreamError, VideoError};

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
    /// ffmpeg exit result, consumed by `take_error` after frame delivery ends.
    completion: Option<oneshot::Receiver<Result<(), MediaError>>>,
    /// Stops the ffmpeg worker when this source drops, including stalled input reads.
    _kill_guard: crate::video::subprocess::KillGuard,
}

impl VideoSource {
    pub(crate) fn new(
        rx: tokio::sync::mpsc::Receiver<RgbFrame>,
        completion: oneshot::Receiver<Result<(), MediaError>>,
        kill_guard: crate::video::subprocess::KillGuard,
    ) -> Self {
        Self {
            rx,
            completion: Some(completion),
            _kill_guard: kill_guard,
        }
    }
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

    /// Returns the video process error after `next` yields `None`.
    /// Static and animated sources return no error; video waits for process completion.
    pub async fn take_error(&mut self) -> Option<StreamError> {
        match self {
            Self::Video(v) => {
                let comp = v.completion.take()?;
                match comp.await {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(StreamError::Video(VideoError::Media(e))),
                    Err(_) => None,
                }
            }
            Self::StaticImage(_) | Self::Animated(_) => None,
        }
    }

    /// Rewinds static and animated sources. Returns `false` for video, whose
    /// repetition requires ffmpeg seeking or rebuilding through the orchestrator.
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
