//! Static image decoding with input, dimension and allocation limits.

use crate::error::{ImageError, MediaError};

pub const MAX_SIZE_LIMIT: usize = 50 * 1024 * 1024;
pub const MEMORY_THRESHOLD: usize = 500 * 1024;
pub const MIN_DELAY_MS: f32 = 10.0;

/// Dimension cap limits decompression-bomb allocations.
pub const MAX_DECODE_DIM: u32 = 8192;

/// Upper bound on decoder scratch allocation.
pub const MAX_DECODE_ALLOC_BYTES: u64 = 256 * 1024 * 1024;

/// Decoded static frame ready for the pipeline.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Raw ICC profile bytes, if the source embedded one.
    pub icc_profile: Option<Vec<u8>>,
}

/// Decodes bounded image bytes to RGBA8, retaining PNG or JPEG ICC metadata.
/// Returns an error for malformed data or exceeded input/decoder limits.
pub fn decode_bytes(data: &[u8], source_url: &str) -> Result<DecodedImage, ImageError> {
    if data.len() > MAX_SIZE_LIMIT {
        return Err(ImageError::DecompressionBomb {
            actual: data.len(),
            limit: MAX_SIZE_LIMIT,
        });
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| {
            ImageError::Media(MediaError::Format {
                source_url: source_url.into(),
                message: e.to_string(),
            })
        })?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIM);
    limits.max_image_height = Some(MAX_DECODE_DIM);
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    reader.limits(limits);
    // A separate decoder preserves ICC metadata that `ImageReader` discards.
    let icc_profile = icc_profile_from_bytes(data);
    let img = reader.decode().map_err(|e| ImageError::Decode(e.to_string()))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(DecodedImage {
        rgba: rgba.into_raw(),
        width: w,
        height: h,
        icc_profile,
    })
}

fn icc_profile_from_bytes(data: &[u8]) -> Option<Vec<u8>> {
    use image::{ImageDecoder, codecs::jpeg::JpegDecoder, codecs::png::PngDecoder};
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        if let Ok(mut d) = PngDecoder::new(std::io::Cursor::new(data))
            && let Ok(Some(p)) = d.icc_profile()
        {
            return Some(p);
        }
        return None;
    }
    // JPEG ICC profiles can span multiple APP2 markers.
    if data.starts_with(&[0xFF, 0xD8]) {
        if let Ok(mut d) = JpegDecoder::new(std::io::Cursor::new(data))
            && let Ok(Some(p)) = d.icc_profile()
        {
            return Some(p);
        }
        return None;
    }
    None
}
