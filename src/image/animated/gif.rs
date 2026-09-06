//! GIF decoding through `gif-dispose`, which owns the canvas and disposal state.

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
        // GIF stores delays in centiseconds.
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

        // `Screen` owns the canvas; copying gives the caller independent frame pixels.
        let rgba_bytes: &[u8] = bytemuck::cast_slice(pixels.buf());
        Ok(Some(AnimatedFrame {
            rgba: rgba_bytes.to_vec(),
            width: w,
            height: h,
            delay_ms,
        }))
    }
}
