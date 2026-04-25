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
    pub fn resize_to_rgb888(&self, dst_w: u32, dst_h: u32) -> Result<Vec<u8>, ImageError> {
        if dst_w == 0 || dst_h == 0 {
            return Err(ImageError::Resize("dst dimensions zero".into()));
        }
        let mut out = vec![0u8; (dst_w * dst_h * 3) as usize];
        let sw = self.width as f64;
        let sh = self.height as f64;
        for y in 0..dst_h {
            for x in 0..dst_w {
                // Nearest-neighbor index lookup.
                let sx = ((x as f64 + 0.5) * sw / dst_w as f64).floor() as u32;
                let sy = ((y as f64 + 0.5) * sh / dst_h as f64).floor() as u32;
                let sx = sx.min(self.width - 1);
                let sy = sy.min(self.height - 1);
                let idx = self.indices[(sy * self.width + sx) as usize];
                let (r, g, b) = if Some(idx) == self.transparency_index {
                    (0, 0, 0)
                } else {
                    let i = (idx as usize).min(self.palette_rgb.len().saturating_sub(1));
                    let rgb = self.palette_rgb[i];
                    (rgb[0], rgb[1], rgb[2])
                };
                let o = ((y * dst_w + x) * 3) as usize;
                out[o] = r;
                out[o + 1] = g;
                out[o + 2] = b;
            }
        }
        Ok(out)
    }
}
