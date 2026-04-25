//! Palette-preserving resize for indexed / transparency-index images.
//!
//! Resize the *indices* with nearest-neighbor, then map back through the
//! palette. Keeps pixel-art LED content crisp and avoids the quantization
//! losses you'd get from resampling RGB.

use crate::error::ImageError;

#[derive(Debug, Clone)]
pub struct Paletted {
    pub indices: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub palette_rgb: Vec<[u8; 3]>,
    pub transparency_index: Option<u8>,
}

impl Paletted {
    /// Resize indices with nearest-neighbor, then expand through the palette.
    ///
    /// Transparent pixels blend onto a black background (LED default).
    /// Source-x and source-y nearest-neighbor maps are precomputed once each
    /// — the inner loop is two indexed loads and three byte writes per pixel.
    pub fn resize_to_rgb888(&self, dst_w: u32, dst_h: u32) -> Result<Vec<u8>, ImageError> {
        if dst_w == 0 || dst_h == 0 {
            return Err(ImageError::Resize("dst dimensions zero".into()));
        }
        let sw = self.width as f64;
        let sh = self.height as f64;
        let last_x = self.width.saturating_sub(1);
        let last_y = self.height.saturating_sub(1);
        let sx_table: Vec<u32> = (0..dst_w)
            .map(|x| (((x as f64 + 0.5) * sw / dst_w as f64).floor() as u32).min(last_x))
            .collect();
        let sy_table: Vec<u32> = (0..dst_h)
            .map(|y| (((y as f64 + 0.5) * sh / dst_h as f64).floor() as u32).min(last_y))
            .collect();

        // Pre-expand the palette to RGB888 with the transparency index
        // collapsed to black, so the inner loop has no per-pixel branches.
        let palette_size = self.palette_rgb.len().max(1);
        let mut palette_rgb888 = vec![0u8; palette_size * 3];
        for (i, rgb) in self.palette_rgb.iter().enumerate() {
            let off = i * 3;
            palette_rgb888[off] = rgb[0];
            palette_rgb888[off + 1] = rgb[1];
            palette_rgb888[off + 2] = rgb[2];
        }
        if let Some(t) = self.transparency_index {
            let i = (t as usize).min(palette_size - 1);
            let off = i * 3;
            palette_rgb888[off] = 0;
            palette_rgb888[off + 1] = 0;
            palette_rgb888[off + 2] = 0;
        }
        let palette_clamp = palette_size - 1;

        let mut out = vec![0u8; (dst_w * dst_h * 3) as usize];
        let src_w_us = self.width as usize;
        for (y, &sy) in sy_table.iter().enumerate() {
            let row_base = (sy as usize) * src_w_us;
            let dst_row_off = y * (dst_w as usize) * 3;
            for (x, &sx) in sx_table.iter().enumerate() {
                let idx = self.indices[row_base + sx as usize] as usize;
                let pal_off = idx.min(palette_clamp) * 3;
                let dst_off = dst_row_off + x * 3;
                out[dst_off] = palette_rgb888[pal_off];
                out[dst_off + 1] = palette_rgb888[pal_off + 1];
                out[dst_off + 2] = palette_rgb888[pal_off + 2];
            }
        }
        Ok(out)
    }
}
