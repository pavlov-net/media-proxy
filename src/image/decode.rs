//! Fetch + decode image bytes.
//!
//! Size caps: >50 MB rejected pre-decode; ≤500 KB held in memory, larger
//! payloads spill to a temp file.

use crate::error::{ImageError, MediaError};

pub const MAX_SIZE_LIMIT: usize = 50 * 1024 * 1024;
pub const MEMORY_THRESHOLD: usize = 500 * 1024;
pub const MIN_DELAY_MS: f32 = 10.0;

/// Upper bound on decoded image dimensions. Anything larger is refused to
/// prevent decompression-bomb blowups in the decoder.
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

/// Decode a byte buffer into RGBA. Returns an [`ImageError`] on malformed
/// data; the caller is responsible for size-capping upstream.
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
    // Extract ICC profile via a per-format decoder pass (the generic
    // `ImageReader` drops it). Currently supported: PNG, JPEG, WebP.
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
    // PNG: iCCP chunk. Try the png decoder first when the signature matches.
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        if let Ok(mut d) = PngDecoder::new(std::io::Cursor::new(data))
            && let Ok(Some(p)) = d.icc_profile()
        {
            return Some(p);
        }
        return None;
    }
    // JPEG: APP2 markers joined from multi-segment `ICC_PROFILE`.
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
