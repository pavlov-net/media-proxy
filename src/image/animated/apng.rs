//! APNG decoder — the `png` crate gives us per-frame `FrameControl`
//! (dispose_op, blend_op, offset x/y, size); we run the compositor manually.
//!
//! Disposal semantics (APNG spec):
//! - 0 (NONE)        : leave frame on canvas before next.
//! - 1 (BACKGROUND)  : clear the rect to transparent black before next.
//! - 2 (PREVIOUS)    : restore canvas to pre-frame state before next.
//!
//! Blend semantics:
//! - 0 (SOURCE): replace rect pixels entirely.
//! - 1 (OVER)  : alpha-blend over existing pixels.

use png::{BlendOp, ColorType, DisposeOp};

use super::{AnimatedFrame, DEFAULT_DELAY_MS, MIN_DELAY_MS};
use crate::error::ImageError;

pub struct ApngDecoder {
    reader: png::Reader<std::io::Cursor<Vec<u8>>>,
    canvas: Vec<u8>, // RGBA, row-major at (width × height)
    previous_canvas: Vec<u8>,
    width: u32,
    height: u32,
    frames_read: u32,
    frame_count: Option<u32>,
    /// Scratch buffer for decoding one sub-frame.
    sub_buf: Vec<u8>,
    /// Persistent scratch for sub-frame expansion to RGBA8.
    rgba_sub: Vec<u8>,
}

impl ApngDecoder {
    pub fn new(data: Vec<u8>, _source_url: &str) -> Result<Self, ImageError> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|e| ImageError::Decode(format!("png: {e}")))?;
        let info = reader.info();
        let width = info.width;
        let height = info.height;
        let frame_count = info.animation_control.map(|ac| ac.num_frames);

        if width > crate::image::decode::MAX_DECODE_DIM || height > crate::image::decode::MAX_DECODE_DIM {
            return Err(ImageError::Decode(format!(
                "apng: dimensions {width}x{height} exceed cap"
            )));
        }
        // Pixel-count computed in u64 to dodge u32 overflow; cap at 128 Mpx
        // (512 MB of RGBA) which is still an order of magnitude beyond any
        // real-world animated PNG we'd stream to an LED panel.
        let pixels = u64::from(width) * u64::from(height);
        if pixels > 128 * 1024 * 1024 {
            return Err(ImageError::Decode("apng: canvas too large".into()));
        }
        let canvas_bytes = (pixels * 4) as usize;

        let buf_size = reader
            .output_buffer_size()
            .ok_or_else(|| ImageError::Decode("png: output buffer size unknown".into()))?;
        let mut sub_buf = vec![0u8; buf_size];
        // An APNG may have a fallback PNG that is not part of the animation.
        // Its IDAT precedes the first fcTL; consume it without emitting a frame.
        if frame_count.is_some() && reader.info().frame_control.is_none() {
            reader
                .next_frame(&mut sub_buf)
                .map_err(|e| ImageError::Decode(format!("apng default image: {e}")))?;
        }
        Ok(Self {
            canvas: vec![0u8; canvas_bytes],
            previous_canvas: vec![0u8; canvas_bytes],
            width,
            height,
            frames_read: 0,
            frame_count,
            sub_buf,
            rgba_sub: Vec::new(),
            reader,
        })
    }

    fn composite_frame(&mut self, fc: png::FrameControl, rgba_sub: &[u8]) {
        let (x, y) = (fc.x_offset, fc.y_offset);
        let (w, h) = (fc.width, fc.height);
        let canvas_w = self.width;
        let canvas_h = self.height;

        // Visible row/col span after clamping to the canvas.
        let copy_h = h.min(canvas_h.saturating_sub(y));
        let copy_w = w.min(canvas_w.saturating_sub(x));
        if copy_h == 0 || copy_w == 0 {
            return;
        }

        let row_bytes = (copy_w * 4) as usize;

        for row in 0..copy_h {
            let sub_off = (row * w * 4) as usize;
            let sub_row = &rgba_sub[sub_off..sub_off + row_bytes];
            let canvas_off = (((y + row) * canvas_w + x) * 4) as usize;
            let canvas_row = &mut self.canvas[canvas_off..canvas_off + row_bytes];

            match fc.blend_op {
                BlendOp::Source => {
                    canvas_row.copy_from_slice(sub_row);
                }
                BlendOp::Over => composite_row_over(canvas_row, sub_row),
            }
        }
    }

    fn apply_dispose(&mut self, fc: png::FrameControl) {
        match fc.dispose_op {
            DisposeOp::None => {}
            DisposeOp::Background => {
                let (x, y, w, h) = (fc.x_offset, fc.y_offset, fc.width, fc.height);
                // x/y beyond the canvas → nothing to clear (defensive; the
                // `png` crate rejects this, but belt-and-suspenders).
                let Some(remaining_w) = self.width.checked_sub(x) else {
                    return;
                };
                for row in 0..h {
                    let canvas_y = y + row;
                    if canvas_y >= self.height {
                        break;
                    }
                    let start = ((canvas_y * self.width + x) * 4) as usize;
                    let end = start + (w.min(remaining_w) * 4) as usize;
                    self.canvas[start..end].fill(0);
                }
            }
            DisposeOp::Previous => {
                self.canvas.copy_from_slice(&self.previous_canvas);
            }
        }
    }
}

impl ApngDecoder {
    pub fn next_frame(&mut self) -> Result<Option<AnimatedFrame>, ImageError> {
        let Some(limit) = self.frame_count else {
            return Ok(None);
        };
        if self.frames_read >= limit {
            return Ok(None);
        }

        let info = self
            .reader
            .next_frame(&mut self.sub_buf)
            .map_err(|e| ImageError::Decode(format!("apng frame: {e}")))?;

        let frame_control = self
            .reader
            .info()
            .frame_control
            .ok_or_else(|| ImageError::Decode("apng: missing frame control".into()))?;

        // Convert sub-frame to RGBA if needed. `png` gives us raw bytes in
        // the source color type; coerce to RGBA8.
        let color = info.color_type;
        let bit_depth = info.bit_depth;
        if bit_depth != png::BitDepth::Eight {
            return Err(ImageError::Decode(format!(
                "apng: unsupported bit depth {bit_depth:?}"
            )));
        }
        let n_pixels = (frame_control.width as usize) * (frame_control.height as usize);
        expand_to_rgba8(
            &self.sub_buf[..info.buffer_size()],
            color,
            n_pixels,
            &mut self.rgba_sub,
        )?;

        // Save "before" state for PREVIOUS disposal.
        if matches!(frame_control.dispose_op, DisposeOp::Previous) {
            self.previous_canvas.copy_from_slice(&self.canvas);
        }

        // Borrow rgba_sub immutably for composite. We re-borrow self.canvas
        // mutably inside composite_frame — split-borrow via a local would
        // help, but we route through composite_frame for clarity.
        let rgba_sub = std::mem::take(&mut self.rgba_sub);
        self.composite_frame(frame_control, &rgba_sub);
        self.rgba_sub = rgba_sub; // hand the buffer back

        let delay_ms = {
            let num = f32::from(frame_control.delay_num);
            let den = if frame_control.delay_den == 0 {
                100.0
            } else {
                f32::from(frame_control.delay_den)
            };
            let raw = num / den * 1000.0;
            if raw <= 0.0 {
                DEFAULT_DELAY_MS
            } else {
                raw.max(MIN_DELAY_MS)
            }
        };

        // Canvas is needed by the next frame's disposal/composite, so copy
        // (don't move) it out.
        let frame = AnimatedFrame {
            rgba: self.canvas.clone(),
            width: self.width,
            height: self.height,
            delay_ms,
        };

        // Apply disposal for the NEXT frame (acts on self.canvas after we
        // snapshotted the current frame into `frame.rgba`).
        self.apply_dispose(frame_control);

        self.frames_read += 1;
        Ok(Some(frame))
    }
}

/// Alpha-blend `src` (RGBA) over `dst` (RGBA), per pixel. Hot inner loop for
/// `BlendOp::Over` — short-circuits the common α=255 (full overwrite) case.
fn composite_row_over(dst: &mut [u8], src: &[u8]) {
    debug_assert_eq!(dst.len(), src.len());
    debug_assert!(dst.len().is_multiple_of(4));
    for (d, s) in dst
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(src.as_chunks::<4>().0.iter())
    {
        let sa = s[3];
        if sa == 255 {
            d.copy_from_slice(s);
            continue;
        }
        if sa == 0 {
            continue;
        }
        // Straight-alpha OVER. Weight destination RGB by its alpha too;
        // otherwise a half-transparent pixel over transparent black darkens
        // twice when the result is later flattened for an LED display.
        let sa = u32::from(sa);
        let da = u32::from(d[3]);
        let inv = 255 - sa;
        let alpha = sa * 255 + da * inv;
        for channel in 0..3 {
            let value = u32::from(s[channel]) * sa * 255 + u32::from(d[channel]) * da * inv;
            d[channel] = ((value + alpha / 2) / alpha) as u8;
        }
        d[3] = ((alpha + 127) / 255) as u8;
    }
}

/// Expand an arbitrary PNG sub-frame buffer to RGBA8 into `out`. APNG
/// sub-frames can be any of the PNG color types; the `png` crate decodes them
/// in their native form. `out` is resized to `n_pixels * 4` bytes.
fn expand_to_rgba8(
    buf: &[u8],
    color: ColorType,
    n_pixels: usize,
    out: &mut Vec<u8>,
) -> Result<(), ImageError> {
    out.resize(n_pixels * 4, 0);
    match color {
        ColorType::Rgba => {
            out.copy_from_slice(&buf[..n_pixels * 4]);
        }
        ColorType::Rgb => {
            for (src, dst) in buf
                .as_chunks::<3>()
                .0
                .iter()
                .zip(out.as_chunks_mut::<4>().0.iter_mut())
            {
                dst[0] = src[0];
                dst[1] = src[1];
                dst[2] = src[2];
                dst[3] = 255;
            }
        }
        ColorType::GrayscaleAlpha => {
            for (src, dst) in buf
                .as_chunks::<2>()
                .0
                .iter()
                .zip(out.as_chunks_mut::<4>().0.iter_mut())
            {
                let (g, a) = (src[0], src[1]);
                dst[0] = g;
                dst[1] = g;
                dst[2] = g;
                dst[3] = a;
            }
        }
        ColorType::Grayscale => {
            for (src, dst) in buf.iter().zip(out.as_chunks_mut::<4>().0.iter_mut()) {
                let g = *src;
                dst[0] = g;
                dst[1] = g;
                dst[2] = g;
                dst[3] = 255;
            }
        }
        ColorType::Indexed => {
            return Err(ImageError::Decode(
                "apng: indexed color not supported (png crate should expand)".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_subframe_on_last_row_composites() {
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, 2, 2);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(2, 0).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0; 16]).unwrap();
            writer.set_frame_dimension(1, 1).unwrap();
            writer.set_frame_position(1, 1).unwrap();
            writer.write_image_data(&[255, 0, 0, 255]).unwrap();
        }
        let mut decoder = ApngDecoder::new(data, "test").unwrap();
        decoder.next_frame().unwrap().unwrap();
        let frame = decoder.next_frame().unwrap().unwrap();
        assert_eq!(&frame.rgba[12..], &[255, 0, 0, 255]);
        assert!(decoder.next_frame().unwrap().is_none());
    }

    #[test]
    fn indexed_apng_expands_palette_and_transparency() {
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, 1, 1);
            encoder.set_color(ColorType::Indexed);
            encoder.set_depth(png::BitDepth::One);
            encoder.set_palette(vec![255, 0, 0, 0, 0, 255]);
            encoder.set_trns(vec![255, 0]);
            encoder.set_animated(1, 0).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0]).unwrap();
        }
        let mut decoder = ApngDecoder::new(data, "test").unwrap();
        assert_eq!(decoder.next_frame().unwrap().unwrap().rgba, [255, 0, 0, 255]);
    }
}
