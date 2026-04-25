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

use bytes::Bytes;
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
}

impl ApngDecoder {
    pub fn new(data: Vec<u8>, _source_url: &str) -> Result<Self, ImageError> {
        let decoder = png::Decoder::new(std::io::Cursor::new(data));
        let reader = decoder
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
        Ok(Self {
            canvas: vec![0u8; canvas_bytes],
            previous_canvas: vec![0u8; canvas_bytes],
            width,
            height,
            frames_read: 0,
            frame_count,
            sub_buf: vec![0u8; buf_size],
            reader,
        })
    }

    fn composite_frame(&mut self, fc: png::FrameControl, rgba_sub: &[u8]) {
        let (x, y) = (fc.x_offset, fc.y_offset);
        let (w, h) = (fc.width, fc.height);

        for row in 0..h {
            let canvas_y = y + row;
            if canvas_y >= self.height {
                break;
            }
            for col in 0..w {
                let canvas_x = x + col;
                if canvas_x >= self.width {
                    break;
                }
                let sub_idx = ((row * w + col) * 4) as usize;
                let canvas_idx = ((canvas_y * self.width + canvas_x) * 4) as usize;
                let src = &rgba_sub[sub_idx..sub_idx + 4];

                match fc.blend_op {
                    BlendOp::Source => {
                        self.canvas[canvas_idx..canvas_idx + 4].copy_from_slice(src);
                    }
                    BlendOp::Over => {
                        let (sr, sg, sb, sa) = (src[0], src[1], src[2], src[3]);
                        if sa == 255 {
                            self.canvas[canvas_idx..canvas_idx + 4].copy_from_slice(src);
                        } else if sa != 0 {
                            let dst = &mut self.canvas[canvas_idx..canvas_idx + 4];
                            let (dr, dg, db, da) = (dst[0], dst[1], dst[2], dst[3]);
                            let sa_u = u16::from(sa);
                            let inv = 255 - sa_u;
                            dst[0] = ((u16::from(sr) * sa_u + u16::from(dr) * inv) / 255) as u8;
                            dst[1] = ((u16::from(sg) * sa_u + u16::from(dg) * inv) / 255) as u8;
                            dst[2] = ((u16::from(sb) * sa_u + u16::from(db) * inv) / 255) as u8;
                            dst[3] = ((sa_u * 255 + u16::from(da) * inv) / 255) as u8;
                        }
                    }
                }
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
        let rgba_sub = to_rgba8(&self.sub_buf[..info.buffer_size()], color)?;

        // Save "before" state for PREVIOUS disposal.
        if matches!(frame_control.dispose_op, DisposeOp::Previous) {
            self.previous_canvas.copy_from_slice(&self.canvas);
        }

        self.composite_frame(frame_control, &rgba_sub);

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

        let frame = AnimatedFrame {
            rgba: Bytes::copy_from_slice(&self.canvas),
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

/// Expand an arbitrary PNG sub-frame buffer to RGBA8. APNG sub-frames can be
/// any of the PNG color types; the `png` crate decodes them in their native
/// form.
fn to_rgba8(buf: &[u8], color: ColorType) -> Result<Vec<u8>, ImageError> {
    match color {
        ColorType::Rgba => Ok(buf.to_vec()),
        ColorType::Rgb => {
            let mut out = Vec::with_capacity(buf.len() / 3 * 4);
            for chunk in buf.chunks_exact(3) {
                out.extend_from_slice(chunk);
                out.push(255);
            }
            Ok(out)
        }
        ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(buf.len() * 2);
            for chunk in buf.chunks_exact(2) {
                let (g, a) = (chunk[0], chunk[1]);
                out.extend_from_slice(&[g, g, g, a]);
            }
            Ok(out)
        }
        ColorType::Grayscale => {
            let mut out = Vec::with_capacity(buf.len() * 4);
            for &g in buf {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            Ok(out)
        }
        ColorType::Indexed => Err(ImageError::Decode(
            "apng: indexed color not supported (png crate should expand)".into(),
        )),
    }
}
