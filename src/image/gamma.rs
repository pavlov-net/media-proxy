//! Lazy lookup tables convert between sRGB u8 and linear-light u16 values.

use std::sync::LazyLock;

pub static SRGB_TO_LINEAR_U16: LazyLock<[u16; 256]> = LazyLock::new(|| {
    let mut lut = [0u16; 256];
    for (i, entry) in lut.iter_mut().enumerate() {
        let x = i as f64 / 255.0;
        let linear = if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        };
        *entry = (linear * 65535.0 + 0.5).clamp(0.0, 65535.0) as u16;
    }
    lut
});

pub static LINEAR_TO_SRGB_U8: LazyLock<Box<[u8; 65536]>> = LazyLock::new(|| {
    let mut lut: Box<[u8; 65536]> = vec![0u8; 65536]
        .into_boxed_slice()
        .try_into()
        .unwrap_or_else(|_| unreachable!("box of 65536 u8"));
    for (i, entry) in lut.iter_mut().enumerate() {
        let y = i as f64 / 65535.0;
        let srgb = if y <= 0.003_130_8 {
            y * 12.92
        } else {
            1.055 * y.powf(1.0 / 2.4) - 0.055
        };
        *entry = (srgb * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    }
    lut
});

/// Writes linear-light values to an output slice of matching length.
pub fn srgb_to_linear(input: &[u8], output: &mut [u16]) {
    debug_assert_eq!(input.len(), output.len());
    for (i, &x) in input.iter().enumerate() {
        output[i] = SRGB_TO_LINEAR_U16[x as usize];
    }
}

/// Writes sRGB values to an output slice of matching length.
pub fn linear_to_srgb(input: &[u16], output: &mut [u8]) {
    debug_assert_eq!(input.len(), output.len());
    for (i, &y) in input.iter().enumerate() {
        output[i] = LINEAR_TO_SRGB_U8[y as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_edges() {
        assert_eq!(SRGB_TO_LINEAR_U16[0], 0);
        assert_eq!(SRGB_TO_LINEAR_U16[255], 65535);
        assert_eq!(LINEAR_TO_SRGB_U8[0], 0);
        assert_eq!(LINEAR_TO_SRGB_U8[65535], 255);
    }

    #[test]
    fn roundtrip_midtones_within_1_lsb() {
        for i in 0u8..=255 {
            let linear = SRGB_TO_LINEAR_U16[i as usize];
            let back = LINEAR_TO_SRGB_U8[linear as usize];
            let delta = (back as i32 - i as i32).abs();
            assert!(delta <= 1, "i={i} linear={linear} back={back}");
        }
    }
}
