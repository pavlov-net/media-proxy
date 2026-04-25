//! Animated WebP via `image-webp` (git `main` pin for post-#179 fixes).
//!
//! `WebPDecoder::read_frame` returns the already-composited canvas per frame,
//! honoring disposal + blend from the VP8X/ANIM chunks.

use bytes::Bytes;
use image_webp::{DecodingError, WebPDecoder};

use super::{AnimatedFrame, DEFAULT_DELAY_MS, MIN_DELAY_MS};
use crate::error::ImageError;

pub struct WebpDecoder {
    decoder: WebPDecoder<std::io::Cursor<Vec<u8>>>,
    width: u32,
    height: u32,
    frame_count: Option<u32>,
    frames_read: u32,
    buf: Vec<u8>,
}

impl WebpDecoder {
    pub fn new(data: Vec<u8>, _source_url: &str) -> Result<Self, ImageError> {
        let decoder = WebPDecoder::new(std::io::Cursor::new(data))
            .map_err(|e| ImageError::Decode(format!("webp: {e}")))?;
        let (width, height) = decoder.dimensions();
        if width > crate::image::decode::MAX_DECODE_DIM || height > crate::image::decode::MAX_DECODE_DIM {
            return Err(ImageError::Decode(format!(
                "webp: dimensions {width}x{height} exceed cap"
            )));
        }
        let pixels = u64::from(width) * u64::from(height);
        if pixels > 128 * 1024 * 1024 {
            return Err(ImageError::Decode("webp: canvas too large".into()));
        }
        let frame_count = if decoder.is_animated() {
            Some(decoder.num_frames())
        } else {
            None
        };
        let buf = vec![0u8; (pixels * 4) as usize];
        Ok(Self {
            decoder,
            width,
            height,
            frame_count,
            frames_read: 0,
            buf,
        })
    }
}

impl WebpDecoder {
    pub fn next_frame(&mut self) -> Result<Option<AnimatedFrame>, ImageError> {
        let Some(limit) = self.frame_count else {
            if self.frames_read == 0 {
                // Single-frame WebP decoded through the animated path —
                // read_image once and return.
                self.decoder.read_image(&mut self.buf).map_err(webp_err)?;
                self.frames_read = 1;
                return Ok(Some(AnimatedFrame {
                    rgba: Bytes::copy_from_slice(&self.buf),
                    width: self.width,
                    height: self.height,
                    delay_ms: DEFAULT_DELAY_MS,
                }));
            }
            return Ok(None);
        };

        if self.frames_read >= limit {
            return Ok(None);
        }

        let duration_ms = self.decoder.read_frame(&mut self.buf).map_err(webp_err)?;
        let delay_ms = {
            let raw = duration_ms as f32;
            if raw <= 0.0 {
                DEFAULT_DELAY_MS
            } else {
                raw.max(MIN_DELAY_MS)
            }
        };

        self.frames_read += 1;
        Ok(Some(AnimatedFrame {
            rgba: Bytes::copy_from_slice(&self.buf),
            width: self.width,
            height: self.height,
            delay_ms,
        }))
    }
}

fn webp_err(e: DecodingError) -> ImageError {
    ImageError::Decode(format!("webp: {e}"))
}
