//! Pick the right decoder by format sniff, composite all frames through the
//! still-image pipeline, populate the frame cache, return the sequence.

use std::sync::Arc;

use bytes::Bytes;

use super::apng::ApngDecoder;
use super::cache::{CacheKey, CachedSequence, FrameCache};
use super::gif::GifDecoder;
use super::webp::WebpDecoder;
use super::{AnimatedDecoder, AnimatedFrame};
use crate::control::fields::Fit;
use crate::error::ImageError;
use crate::image::decode::DecodedImage;
use crate::image::pipeline::{ImagePipeline, PipelineParams};
use crate::image::unsharp::UnsharpParams;

#[derive(Debug, Clone, Copy)]
enum Kind {
    Gif,
    Apng,
    Webp,
    SinglePng,
}

/// Sniff the format from the leading bytes. Indicates whether the buffer is
/// an animated container we handle, or a single-frame format to fall back
/// to the still-image pipeline.
fn sniff(data: &[u8]) -> Option<Kind> {
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some(Kind::Gif);
    }
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        // Look for an `acTL` chunk before `IDAT` to identify APNG.
        let lc = data.len();
        let mut i = 8usize;
        while i + 8 <= lc {
            let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            let kind = &data[i + 4..i + 8];
            if kind == b"acTL" {
                return Some(Kind::Apng);
            }
            if kind == b"IDAT" {
                return Some(Kind::SinglePng);
            }
            i = i.checked_add(8 + len + 4).unwrap_or(lc);
        }
        return Some(Kind::SinglePng);
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some(Kind::Webp);
    }
    None
}

pub fn is_animated(data: &[u8]) -> bool {
    matches!(sniff(data), Some(Kind::Gif | Kind::Apng | Kind::Webp))
}

/// Cap on decoded frames per animated source. A 50 MB file can claim
/// millions of tiny frames; we refuse to composite past this count so the
/// cache + pipeline can't be tricked into eating the heap.
const MAX_ANIMATED_FRAMES: usize = 10_000;

#[derive(Debug)]
pub struct AnimatedDispatchParams<'a> {
    pub target_w: u32,
    pub target_h: u32,
    pub fit: Fit,
    pub method: crate::image::resize::ResampleMethod,
    pub gamma_correct: bool,
    pub color_correction: bool,
    pub unsharp: UnsharpParams,
    pub source_url: &'a str,
    pub r#loop: bool,
}

/// Load, decode, composite, and cache all frames of an animated source.
/// Returns an Arc-shared sequence ready to emit at native cadence.
pub fn dispatch(
    data: Vec<u8>,
    params: &AnimatedDispatchParams,
    cache: &FrameCache,
) -> Result<Arc<CachedSequence>, ImageError> {
    let key = CacheKey {
        url: params.source_url.into(),
        width: params.target_w,
        height: params.target_h,
        fit: params.fit.as_str(),
        method: params.method.as_str(),
    };

    if let Some(seq) = cache.get(&key) {
        tracing::debug!(url = %params.source_url, "animated cache hit");
        return Ok(seq);
    }

    let mut decoder = match sniff(&data) {
        Some(Kind::Gif) => AnimatedDecoder::Gif(Box::new(GifDecoder::new(data, params.source_url)?)),
        Some(Kind::Apng) => AnimatedDecoder::Apng(Box::new(ApngDecoder::new(data, params.source_url)?)),
        Some(Kind::Webp) => AnimatedDecoder::Webp(Box::new(WebpDecoder::new(data, params.source_url)?)),
        Some(Kind::SinglePng) | None => {
            return Err(ImageError::Decode(
                "animated dispatcher called for single-frame input".into(),
            ));
        }
    };

    // Composite each frame through the still-image pipeline so the cached
    // frames are at target size + RGB888.
    let mut composed: Vec<(Bytes, f32)> = Vec::new();
    while let Some(AnimatedFrame {
        rgba,
        width,
        height,
        delay_ms,
    }) = decoder.next_frame()?
    {
        if composed.len() >= MAX_ANIMATED_FRAMES {
            return Err(ImageError::Decode(format!(
                "animated: exceeded max frames ({MAX_ANIMATED_FRAMES})"
            )));
        }
        let decoded = DecodedImage {
            rgba,
            width,
            height,
            icc_profile: None,
        };
        let pipeline = PipelineParams {
            target_w: params.target_w,
            target_h: params.target_h,
            fit: params.fit,
            method: params.method,
            gamma_correct: params.gamma_correct,
            color_correction: params.color_correction,
            unsharp: params.unsharp.clone(),
        };
        let rgb888 = ImagePipeline::run(decoded, &pipeline)?;
        composed.push((Bytes::from(rgb888), delay_ms));
    }

    if composed.is_empty() {
        return Err(ImageError::Decode("animated: no frames decoded".into()));
    }

    let seq = CachedSequence::new(composed);
    if cache.eligible(seq.frames.len(), params.r#loop)
        && let Some(arc) = cache.insert(key, seq.clone())
    {
        return Ok(arc);
    }
    Ok(Arc::new(seq))
}
