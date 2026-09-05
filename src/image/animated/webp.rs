//! Animated WebP via `image-webp` (git `main` pin for post-#179 fixes).
//!
//! `WebPDecoder::read_frame` returns the already-composited canvas per frame,
//! honoring disposal + blend from the VP8X/ANIM chunks.

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
        let size = decoder
            .output_buffer_size()
            .ok_or_else(|| ImageError::Decode("webp: output buffer too large".into()))?;
        let buf = vec![0u8; size];
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
                    rgba: self.take_buf(),
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
            rgba: self.take_buf(),
            width: self.width,
            height: self.height,
            delay_ms,
        }))
    }

    /// Hand the current canvas buffer to the caller and replace it with a
    /// fresh allocation. `read_frame` / `read_image` fully overwrite the
    /// buffer on the next call, so we don't need to preserve its contents.
    fn take_buf(&mut self) -> Vec<u8> {
        if self.decoder.has_alpha() {
            let cap = self.buf.len();
            std::mem::replace(&mut self.buf, vec![0u8; cap])
        } else {
            let mut rgba = Vec::with_capacity(self.width as usize * self.height as usize * 4);
            for pixel in self.buf.as_chunks::<3>().0.iter() {
                rgba.extend_from_slice(pixel);
                rgba.push(255);
            }
            rgba
        }
    }
}

fn webp_err(e: DecodingError) -> ImageError {
    ImageError::Decode(format!("webp: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    #[test]
    fn opaque_webp_expands_rgb_to_rgba() {
        let mut data = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut data)
            .write_image(&[255, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        let mut decoder = WebpDecoder::new(data, "test").unwrap();
        assert_eq!(decoder.next_frame().unwrap().unwrap().rgba, [255, 0, 0, 255]);
        assert!(decoder.next_frame().unwrap().is_none());
    }
}
