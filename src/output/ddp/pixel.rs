//! RGB888 → RGB565 quantization (scalar + SIMD paths, identical output).
//!
//! Per-channel rounding follows `(c / 255) * N + 0.5`, truncated — equivalent
//! to numpy's `(c / 255 * N + 0.5).astype(u16)`. Implemented with integer
//! math so SIMD and scalar paths produce bit-identical output.

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

/// Scalar RGB888 → RGB565 conversion.
///
/// Input must be RGB-triplets (length multiple of 3). Any ragged tail is
/// dropped. Returns a buffer of `(len/3)*2` bytes in the chosen endianness.
pub fn rgb888_to_565(input: &[u8], endian: Endian) -> Vec<u8> {
    let n = input.len() / 3;
    let mut out = vec![0u8; n * 2];
    for i in 0..n {
        let r = input[i * 3] as u32;
        let g = input[i * 3 + 1] as u32;
        let b = input[i * 3 + 2] as u32;

        // (c / 255) * N + 0.5, truncated — integer-only form to keep
        // bit-identical results with any SIMD path.
        let r5 = ((r * 31 * 2 + 255) / (255 * 2)) & 0x1F;
        let g6 = ((g * 63 * 2 + 255) / (255 * 2)) & 0x3F;
        let b5 = ((b * 31 * 2 + 255) / (255 * 2)) & 0x1F;

        let v: u16 = ((r5 as u16) << 11) | ((g6 as u16) << 5) | (b5 as u16);
        let bytes = match endian {
            Endian::Le => v.to_le_bytes(),
            Endian::Be => v.to_be_bytes(),
        };
        out[i * 2] = bytes[0];
        out[i * 2 + 1] = bytes[1];
    }
    out
}

/// Convert a frame to the requested pixel format.
///
/// For `Rgb888`, the input is borrowed unchanged (no copy). For the 565
/// variants, the payload is quantized and re-packed in the requested
/// endianness.
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
