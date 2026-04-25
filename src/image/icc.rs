//! ICC profile → sRGB conversion via `lcms2`.
//!
//! Uses relative colorimetric intent (PIL's default) and degrades gracefully
//! to "do nothing" when the profile is missing/invalid/already-sRGB.

use lcms2::{Intent, PixelFormat, Profile, Transform};
use tracing::{debug, warn};

use crate::error::ImageError;

/// Convert an RGBA buffer from its embedded ICC profile to sRGB. Mutates
/// the buffer in place. Returns `Ok(false)` if no conversion was needed.
pub fn to_srgb_inplace(rgba: &mut [u8], icc: Option<&[u8]>) -> Result<bool, ImageError> {
    let Some(profile_bytes) = icc else {
        return Ok(false);
    };

    let source = Profile::new_icc(profile_bytes)
        .map_err(|e| ImageError::Icc(format!("load embedded profile: {e}")))?;
    let srgb = Profile::new_srgb();

    // If the source claims to be sRGB, skip.
    if let Some(name) = source.info(lcms2::InfoType::Description, lcms2::Locale::none())
        && name.to_ascii_lowercase().contains("srgb")
    {
        debug!("source already sRGB — skipping ICC transform");
        return Ok(false);
    }

    // One pixel = `[u8; 4]`. The transform runs the 4-channel input/output
    // through RGBA_8 → RGBA_8.
    let transform: Transform<[u8; 4], [u8; 4]> = Transform::new(
        &source,
        PixelFormat::RGBA_8,
        &srgb,
        PixelFormat::RGBA_8,
        Intent::RelativeColorimetric,
    )
    .map_err(|e| ImageError::Icc(format!("build transform: {e}")))?;

    // In-place transform over the whole buffer at once.
    let pixel_count = rgba.len() / 4;
    // SAFETY: `rgba` is `len = pixel_count * 4`; `[u8; 4]` has the same
    // layout and alignment as 4×u8, so reinterpreting is sound.
    let pixels: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut rgba[..pixel_count * 4]);
    transform.transform_in_place(pixels);
    Ok(true)
}

/// Quiet variant — logs warnings instead of propagating errors. A broken
/// profile shouldn't take down the stream.
pub fn to_srgb_inplace_soft(rgba: &mut [u8], icc: Option<&[u8]>) -> bool {
    match to_srgb_inplace(rgba, icc) {
        Ok(applied) => applied,
        Err(e) => {
            warn!(error = %e, "ICC conversion failed — leaving buffer untouched");
            false
        }
    }
}
