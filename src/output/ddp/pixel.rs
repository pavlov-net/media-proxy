//! RGB888 to RGB565 quantization with integer per-channel rounding.

use crate::output::sink::PixelFormat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Le,
    Be,
}

pub fn endian_for(format: PixelFormat) -> Option<Endian> {
    match format {
        PixelFormat::Rgb565Le => Some(Endian::Le),
        PixelFormat::Rgb565Be => Some(Endian::Be),
        PixelFormat::Rgb888 => None,
    }
}

/// Precomputed red-channel rounding and placement in RGB565.
static LUT_R5_SHIFTED: [u16; 256] = build_r5_lut();
static LUT_G6_SHIFTED: [u16; 256] = build_g6_lut();
static LUT_B5: [u16; 256] = build_b5_lut();

const fn build_r5_lut() -> [u16; 256] {
    let mut lut = [0u16; 256];
    let mut i = 0u32;
    while i < 256 {
        lut[i as usize] = (((i * 31 * 2 + 255) / 510) as u16) << 11;
        i += 1;
    }
    lut
}

const fn build_g6_lut() -> [u16; 256] {
    let mut lut = [0u16; 256];
    let mut i = 0u32;
    while i < 256 {
        lut[i as usize] = (((i * 63 * 2 + 255) / 510) as u16) << 5;
        i += 1;
    }
    lut
}

const fn build_b5_lut() -> [u16; 256] {
    let mut lut = [0u16; 256];
    let mut i = 0u32;
    while i < 256 {
        lut[i as usize] = ((i * 31 * 2 + 255) / 510) as u16;
        i += 1;
    }
    lut
}

/// Converts RGB triplets to RGB565, dropping any incomplete trailing pixel.
/// Returns `(len / 3) * 2` bytes in the requested byte order.
pub fn rgb888_to_565(input: &[u8], endian: Endian) -> Vec<u8> {
    let n = input.len() / 3;
    let mut out = vec![0u8; n * 2];
    match endian {
        Endian::Le => convert_to(input, &mut out, |v| v.to_le_bytes()),
        Endian::Be => convert_to(input, &mut out, |v| v.to_be_bytes()),
    }
    out
}

#[inline]
fn convert_to(input: &[u8], out: &mut [u8], to_bytes: fn(u16) -> [u8; 2]) {
    for (i_chunk, o_chunk) in input
        .as_chunks::<3>()
        .0
        .iter()
        .zip(out.as_chunks_mut::<2>().0.iter_mut())
    {
        let v = LUT_R5_SHIFTED[i_chunk[0] as usize]
            | LUT_G6_SHIFTED[i_chunk[1] as usize]
            | LUT_B5[i_chunk[2] as usize];
        let bytes = to_bytes(v);
        o_chunk[0] = bytes[0];
        o_chunk[1] = bytes[1];
    }
}

/// Converts a frame to the requested format, borrowing RGB888 unchanged
/// and allocating packed bytes for RGB565.
pub fn encode_frame<'a>(input: &'a [u8], format: PixelFormat) -> std::borrow::Cow<'a, [u8]> {
    use std::borrow::Cow;
    match format {
        PixelFormat::Rgb888 => Cow::Borrowed(input),
        PixelFormat::Rgb565Le => Cow::Owned(rgb888_to_565(input, Endian::Le)),
        PixelFormat::Rgb565Be => Cow::Owned(rgb888_to_565(input, Endian::Be)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_maps_to_0x0000() {
        let out = rgb888_to_565(&[0, 0, 0], Endian::Le);
        assert_eq!(u16::from_le_bytes([out[0], out[1]]), 0);
    }

    #[test]
    fn white_maps_to_0xffff() {
        let out = rgb888_to_565(&[255, 255, 255], Endian::Le);
        assert_eq!(u16::from_le_bytes([out[0], out[1]]), 0xFFFF);
    }

    #[test]
    fn red_component_occupies_upper_5() {
        let out = rgb888_to_565(&[255, 0, 0], Endian::Le);
        let v = u16::from_le_bytes([out[0], out[1]]);
        assert_eq!(v >> 11, 0x1F);
        assert_eq!(v & 0x07FF, 0);
    }

    #[test]
    fn green_component_occupies_middle_6() {
        let out = rgb888_to_565(&[0, 255, 0], Endian::Le);
        let v = u16::from_le_bytes([out[0], out[1]]);
        assert_eq!((v >> 5) & 0x3F, 0x3F);
    }

    #[test]
    fn blue_component_occupies_lower_5() {
        let out = rgb888_to_565(&[0, 0, 255], Endian::Le);
        let v = u16::from_le_bytes([out[0], out[1]]);
        assert_eq!(v & 0x1F, 0x1F);
    }

    #[test]
    fn be_le_differ_only_in_byte_order() {
        let input = [0xAB, 0xCD, 0xEF];
        let le = rgb888_to_565(&input, Endian::Le);
        let be = rgb888_to_565(&input, Endian::Be);
        assert_eq!([le[0], le[1]], [be[1], be[0]]);
    }

    #[test]
    fn ragged_tail_truncated() {
        // 4 bytes = 1 complete pixel + 1 ragged byte
        let out = rgb888_to_565(&[0, 0, 0, 0xFF], Endian::Le);
        assert_eq!(out.len(), 2);
    }
}
