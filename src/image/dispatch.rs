//! Fetches images, selects static or animated decoding, and builds a frame source.

use crate::Config;
use crate::control::fields::StreamFields;
use crate::error::StreamError;
use crate::image::animated::cache::FrameCache;
use crate::image::animated::dispatch::{self as animated, AnimatedDispatchParams};
use crate::image::decode::decode_bytes;
use crate::image::pipeline::{ImagePipeline, PipelineParams};
use crate::image::resize::ResampleMethod;
use crate::image::unsharp::UnsharpParams;
use crate::stream::fetcher;
use crate::stream::frame_source::{AnimatedSource, FrameSource, StaticImageSource};

pub async fn build_image_source(
    fields: &StreamFields,
    config: &Config,
    frame_cache: &FrameCache,
) -> Result<FrameSource, StreamError> {
    let bytes = fetcher::fetch_bytes(&fields.source, &config.net.user_agent)
        .await
        .map_err(StreamError::Image)?;

    let method = ResampleMethod::from_str_canon(&config.image.method);
    let unsharp = UnsharpParams {
        radius: config.image.unsharp.radius,
        amount: config.image.unsharp.amount,
        threshold: config.image.unsharp.threshold,
    };

    if animated::is_animated(&bytes) {
        let params = AnimatedDispatchParams {
            target_w: fields.width,
            target_h: fields.height,
            fit: fields.fit,
            method,
            gamma_correct: config.image.gamma_correct,
            color_correction: config.image.color_correction,
            unsharp,
            source_url: &fields.source,
            r#loop: fields.r#loop,
        };
        // Decoding runs inline on the async task.
        let seq = animated::dispatch(bytes, &params, frame_cache).map_err(StreamError::Image)?;
        return Ok(FrameSource::Animated(AnimatedSource {
            frames: seq,
            cursor: 0,
        }));
    }

    let decoded = decode_bytes(&bytes, &fields.source).map_err(StreamError::Image)?;
    let params = PipelineParams {
        target_w: fields.width,
        target_h: fields.height,
        fit: fields.fit,
        method,
        gamma_correct: config.image.gamma_correct,
        color_correction: config.image.color_correction,
        unsharp,
    };
    let rgb888 = ImagePipeline::run(decoded, &params).map_err(StreamError::Image)?;
    Ok(FrameSource::StaticImage(StaticImageSource {
        frame: bytes::Bytes::from(rgb888),
        emitted: false,
    }))
}
