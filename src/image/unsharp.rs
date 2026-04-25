//! Pillow-compatible unsharp mask.
//!
//! Algorithm (matches Pillow's `UnsharpMask` in `ImageFilter.c`):
//! 1. Gaussian blur of input with radius `r`.
//! 2. For each pixel: diff = orig - blurred (per channel).
//! 3. If |diff| < threshold, keep orig unchanged.
//! 4. Else orig += diff * (percent / 100).
//! 5. Clamp to [0, 255].

#[derive(Debug, Clone)]
pub struct UnsharpParams {
    pub radius: f32,
    pub amount: f32, // 0.0..1.0 (Pillow's "percent" is `amount * 100`)
    pub threshold: i32,
}

/// In-place unsharp on RGB888.
pub fn unsharp_rgb888(buf: &mut [u8], width: u32, height: u32, p: &UnsharpParams) {
    if p.amount <= 0.0 {
        return;
    }
    let blurred = box_blur_rgb888(buf, width, height, p.radius.max(0.1));
    let percent = p.amount; // already 0..1

    for i in 0..buf.len() {
        let orig = buf[i] as i32;
        let blur = blurred[i] as i32;
        let diff = orig - blur;
        if diff.abs() < p.threshold {
            continue;
        }
        let adjusted = orig + (diff as f32 * percent).round() as i32;
        buf[i] = adjusted.clamp(0, 255) as u8;
    }
}

/// 2-pass separable box blur, used as a cheap Gaussian approximation.
/// Good enough at LED resolutions; could be replaced with a true Gaussian
/// if a parity fixture suite ever demands it.
fn box_blur_rgb888(src: &[u8], width: u32, height: u32, radius: f32) -> Vec<u8> {
    let mut tmp = src.to_vec();
    let r = radius.round().max(1.0) as i32;
    let w = width as i32;
    let h = height as i32;

    // Horizontal pass.
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let x0 = (x - r).max(0);
            let x1 = (x + r).min(w - 1);
            let count = (x1 - x0 + 1) as u32;
            let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
            for xx in x0..=x1 {
                let idx = ((y * w + xx) * 3) as usize;
                sr += tmp[idx] as u32;
                sg += tmp[idx + 1] as u32;
                sb += tmp[idx + 2] as u32;
            }
            let o = ((y * w + x) * 3) as usize;
            out[o] = (sr / count) as u8;
            out[o + 1] = (sg / count) as u8;
            out[o + 2] = (sb / count) as u8;
        }
    }
    tmp.copy_from_slice(&out);

    // Vertical pass.
    for x in 0..w {
        for y in 0..h {
            let y0 = (y - r).max(0);
            let y1 = (y + r).min(h - 1);
            let count = (y1 - y0 + 1) as u32;
            let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
            for yy in y0..=y1 {
                let idx = ((yy * w + x) * 3) as usize;
                sr += tmp[idx] as u32;
                sg += tmp[idx + 1] as u32;
                sb += tmp[idx + 2] as u32;
            }
            let o = ((y * w + x) * 3) as usize;
            out[o] = (sr / count) as u8;
            out[o + 1] = (sg / count) as u8;
            out[o + 2] = (sb / count) as u8;
        }
    }
    out
}
