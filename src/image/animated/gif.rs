//! GIF decoder with disposal handling via `gif` + `gif-dispose`.
//!
//! `gif-dispose` 6.x owns the canvas state and handles all three disposal
//! methods (NONE / BACKGROUND / PREVIOUS) — we just feed it decoded frames
//! and it returns the fully-composited canvas.

use gif::Decoder;
use gif_dispose::Screen;

use super::{AnimatedFrame, DEFAULT_DELAY_MS, MIN_DELAY_MS};
use crate::error::{ImageError, MediaError};

pub struct GifDecoder {
    decoder: Decoder<std::io::Cursor<Vec<u8>>>,
    screen: Screen,
}

impl GifDecoder {
    pub fn new(data: Vec<u8>, source_url: &str) -> Result<Self, ImageError> {
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::Indexed);
        let decoder = opts.read_info(std::io::Cursor::new(data)).map_err(|e| {
            ImageError::Media(MediaError::Format {
                source_url: source_url.into(),
                message: format!("gif: {e}"),
            })
        })?;
        let screen = Screen::new_decoder(&decoder);
        Ok(Self { decoder, screen })
    }

    pub fn next_frame(&mut self) -> Result<Option<AnimatedFrame>, ImageError> {
        let frame = match self.decoder.read_next_frame() {
            Ok(Some(f)) => f,
            Ok(None) => return Ok(None),
            Err(e) => return Err(ImageError::Decode(format!("gif frame: {e}"))),
        };
        // GIF delay is in centiseconds; clamp to MIN_DELAY_MS so downstream
        // pacing never sees a sub-frame interval.
        let delay_ms = {
            let raw = f32::from(frame.delay) * 10.0;
            if raw <= 0.0 {
                DEFAULT_DELAY_MS
            } else {
                raw.max(MIN_DELAY_MS)
            }
        };

        self.screen
            .blit_frame(frame)
            .map_err(|e| ImageError::Decode(format!("gif blit: {e}")))?;
        let pixels = self.screen.pixels_rgba();
        let (w, h) = (pixels.width() as u32, pixels.height() as u32);

        // `pixels_rgba()` returns `ImgVec<RGBA8>`; flatten to `&[u8]` via
        // bytemuck since `RGBA8` is `#[repr(C)]` Pod.
        let rgba_bytes: &[u8] = bytemuck::cast_slice(pixels.buf());
        Ok(Some(AnimatedFrame {
            rgba: bytes::Bytes::copy_from_slice(rgba_bytes),
            width: w,
            height: h,
            delay_ms,
        }))
    }
}
