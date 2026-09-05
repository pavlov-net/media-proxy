//! RGB888 sharpening with a separable box blur and a per-channel threshold.

#[derive(Debug, Clone)]
pub struct UnsharpParams {
    pub radius: f32,
    pub amount: f32, // Gain multiplier; nonpositive values disable sharpening.
    pub threshold: i32,
}

/// In-place unsharp on RGB888.
pub fn unsharp_rgb888(buf: &mut [u8], width: u32, height: u32, p: &UnsharpParams) {
    if p.amount <= 0.0 {
        return;
    }
    let blurred = box_blur_rgb888(buf, width, height, p.radius.max(0.1));
    let percent = p.amount;

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

/// Computes a separable box blur in O(width * height).
/// Edge windows use only in-bounds pixels; each pass truncates its average.
fn box_blur_rgb888(src: &[u8], width: u32, height: u32, radius: f32) -> Vec<u8> {
    let r = radius.round().max(1.0) as i32;
    let w = width as i32;
    let h = height as i32;
    let stride = (w as usize) * 3;

    // Every temporary pixel is written before the vertical pass reads it.
    let mut tmp = vec![0u8; src.len()];
    for y in 0..h as usize {
        let row_in = &src[y * stride..(y + 1) * stride];
        let row_out = &mut tmp[y * stride..(y + 1) * stride];
        blur_row(row_in, row_out, w, r);
    }

    let mut out = vec![0u8; src.len()];
    let mut sums = vec![0u32; (w as usize) * 3];
    let r_us = r as usize;
    let h_us = h as usize;

    // The first window includes rows 0 through r, clipped to the image.
    let init_end = r_us.min(h_us - 1);
    for yy in 0..=init_end {
        add_row_to_sums(&mut sums, &tmp[yy * stride..(yy + 1) * stride]);
    }
    let mut count = (init_end as u32) + 1;

    for y in 0..h_us {
        write_quotient_row(&sums, &mut out[y * stride..(y + 1) * stride], count);
        let incoming = y as i32 + r + 1;
        if incoming < h {
            add_row_to_sums(
                &mut sums,
                &tmp[(incoming as usize) * stride..(incoming as usize + 1) * stride],
            );
            count += 1;
        }
        let outgoing = y as i32 - r;
        if outgoing >= 0 {
            sub_row_from_sums(
                &mut sums,
                &tmp[(outgoing as usize) * stride..(outgoing as usize + 1) * stride],
            );
            count -= 1;
        }
    }
    out
}

/// Adds a row to per-channel sums using eight SIMD lanes and a scalar tail.
fn add_row_to_sums(sums: &mut [u32], row: &[u8]) {
    debug_assert_eq!(sums.len(), row.len());
    let n = sums.len();
    let main_n = n - (n % 8);
    let mut i = 0;
    while i < main_n {
        let row_v = wide::u32x8::new([
            u32::from(row[i]),
            u32::from(row[i + 1]),
            u32::from(row[i + 2]),
            u32::from(row[i + 3]),
            u32::from(row[i + 4]),
            u32::from(row[i + 5]),
            u32::from(row[i + 6]),
            u32::from(row[i + 7]),
        ]);
        let sums_v = wide::u32x8::new([
            sums[i],
            sums[i + 1],
            sums[i + 2],
            sums[i + 3],
            sums[i + 4],
            sums[i + 5],
            sums[i + 6],
            sums[i + 7],
        ]);
        let r = sums_v + row_v;
        sums[i..i + 8].copy_from_slice(&r.to_array());
        i += 8;
    }
    while i < n {
        sums[i] += u32::from(row[i]);
        i += 1;
    }
}

/// Subtracts a row from per-channel sums using eight SIMD lanes and a scalar tail.
fn sub_row_from_sums(sums: &mut [u32], row: &[u8]) {
    debug_assert_eq!(sums.len(), row.len());
    let n = sums.len();
    let main_n = n - (n % 8);
    let mut i = 0;
    while i < main_n {
        let row_v = wide::u32x8::new([
            u32::from(row[i]),
            u32::from(row[i + 1]),
            u32::from(row[i + 2]),
            u32::from(row[i + 3]),
            u32::from(row[i + 4]),
            u32::from(row[i + 5]),
            u32::from(row[i + 6]),
            u32::from(row[i + 7]),
        ]);
        let sums_v = wide::u32x8::new([
            sums[i],
            sums[i + 1],
            sums[i + 2],
            sums[i + 3],
            sums[i + 4],
            sums[i + 5],
            sums[i + 6],
            sums[i + 7],
        ]);
        let r = sums_v - row_v;
        sums[i..i + 8].copy_from_slice(&r.to_array());
        i += 8;
    }
    while i < n {
        sums[i] -= u32::from(row[i]);
        i += 1;
    }
}

/// Writes truncated per-channel averages; `wide` has no integer division.
fn write_quotient_row(sums: &[u32], row_out: &mut [u8], count: u32) {
    debug_assert_eq!(sums.len(), row_out.len());
    for (s, r) in sums.iter().zip(row_out.iter_mut()) {
        *r = (s / count) as u8;
    }
}

/// Blurs one row with a 1-D sliding window of half-width `r`, edge-clamped.
fn blur_row(row_in: &[u8], row_out: &mut [u8], w: i32, r: i32) {
    let w_us = w as usize;
    let r_us = r as usize;
    let init_end = r_us.min(w_us - 1);
    let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
    for xx in 0..=init_end {
        sr += u32::from(row_in[xx * 3]);
        sg += u32::from(row_in[xx * 3 + 1]);
        sb += u32::from(row_in[xx * 3 + 2]);
    }
    let mut count = (init_end as u32) + 1;

    for x in 0..w_us {
        row_out[x * 3] = (sr / count) as u8;
        row_out[x * 3 + 1] = (sg / count) as u8;
        row_out[x * 3 + 2] = (sb / count) as u8;
        let incoming = x as i32 + r + 1;
        if incoming < w {
            let i = (incoming as usize) * 3;
            sr += u32::from(row_in[i]);
            sg += u32::from(row_in[i + 1]);
            sb += u32::from(row_in[i + 2]);
            count += 1;
        }
        let outgoing = x as i32 - r;
        if outgoing >= 0 {
            let i = (outgoing as usize) * 3;
            sr -= u32::from(row_in[i]);
            sg -= u32::from(row_in[i + 1]);
            sb -= u32::from(row_in[i + 2]);
            count -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Direct window summation provides an independent blur reference.
    fn box_blur_naive(src: &[u8], width: u32, height: u32, radius: f32) -> Vec<u8> {
        let mut tmp = src.to_vec();
        let r = radius.round().max(1.0) as i32;
        let w = width as i32;
        let h = height as i32;
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

    fn rand_image(w: u32, h: u32, seed: u64) -> Vec<u8> {
        let mut s = seed;
        let n = (w * h * 3) as usize;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            v.push((s >> 33) as u8);
        }
        v
    }

    #[test]
    fn sliding_window_matches_naive_for_various_sizes_and_radii() {
        for &(w, h) in &[(1, 1), (2, 3), (4, 4), (16, 9), (33, 17), (64, 64)] {
            for &r in &[1.0f32, 2.0, 3.0, 5.0] {
                let src = rand_image(w, h, (w as u64) * 1000 + h as u64 + (r as u64));
                let fast = box_blur_rgb888(&src, w, h, r);
                let slow = box_blur_naive(&src, w, h, r);
                assert_eq!(fast, slow, "blur mismatch w={w} h={h} r={r}");
            }
        }
    }
}
