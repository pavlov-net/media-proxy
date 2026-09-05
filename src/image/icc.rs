//! ICC conversion to sRGB with relative colorimetric intent.
//! `to_srgb_inplace_soft` logs conversion failures and leaves pixels unchanged.

use lcms2::{Intent, PixelFormat, Profile, Transform};
use tracing::{debug, warn};

use crate::error::ImageError;

/// Converts RGBA pixels to sRGB in place. Returns `Ok(false)` for absent
/// profiles or descriptions containing "srgb"; invalid profiles return an error.
pub fn to_srgb_inplace(rgba: &mut [u8], icc: Option<&[u8]>) -> Result<bool, ImageError> {
    let Some(profile_bytes) = icc else {
        return Ok(false);
    };

    let source = Profile::new_icc(profile_bytes)
        .map_err(|e| ImageError::Icc(format!("load embedded profile: {e}")))?;
    let srgb = Profile::new_srgb();

    if let Some(name) = source.info(lcms2::InfoType::Description, lcms2::Locale::none())
        && name.to_ascii_lowercase().contains("srgb")
    {
        debug!("source already sRGB — skipping ICC transform");
        return Ok(false);
    }

    let transform: Transform<[u8; 4], [u8; 4]> = Transform::new(
        &source,
        PixelFormat::RGBA_8,
        &srgb,
        PixelFormat::RGBA_8,
        Intent::RelativeColorimetric,
    )
    .map_err(|e| ImageError::Icc(format!("build transform: {e}")))?;

    let pixel_count = rgba.len() / 4;
    let pixels: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut rgba[..pixel_count * 4]);
    transform.transform_in_place(pixels);
    Ok(true)
}

/// Logs ICC conversion errors so an invalid profile does not stop playback.
pub fn to_srgb_inplace_soft(rgba: &mut [u8], icc: Option<&[u8]>) -> bool {
    match to_srgb_inplace(rgba, icc) {
        Ok(applied) => applied,
        Err(e) => {
            warn!(error = %e, "ICC conversion failed — leaving buffer untouched");
            false
        }
    }
}
