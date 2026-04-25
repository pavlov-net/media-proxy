//! `POST /api/convert/animimg` — ZIP of PNG frames + ESPHome YAML for
//! LVGL animimg widgets.

use std::io::Cursor;
use std::io::Write;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use image::{ImageEncoder, codecs::png::PngEncoder};
use serde::Deserialize;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::api::AppState;
use crate::control::fields::Fit;
use crate::image::animated::cache::FrameCache;
use crate::image::animated::dispatch::{self, AnimatedDispatchParams};
use crate::image::decode::decode_bytes;
use crate::image::pipeline::{ImagePipeline, PipelineParams};
use crate::image::resize::ResampleMethod;
use crate::image::unsharp::UnsharpParams;
use crate::stream::fetcher;

#[derive(Debug, Deserialize)]
pub struct AnimimgRequest {
    pub source: String,
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_frame_limit")]
    pub frame_limit: u32,
    #[serde(default)]
    pub fps_limit: Option<f32>,
    #[serde(default = "default_fit")]
    pub fit: String,
}

fn default_frame_limit() -> u32 {
    100
}
fn default_fit() -> String {
    "cover".into()
}

pub async fn convert(State(state): State<Arc<AppState>>, Json(req): Json<AnimimgRequest>) -> Response {
    let fit = match req.fit.as_str() {
        "pad" => Fit::Pad,
        "auto" => Fit::Auto,
        _ => Fit::Cover,
    };
    match build_zip(state, &req, fit).await {
        Ok(zip) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
            headers.insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=animimg_frames.zip"),
            );
            (headers, zip).into_response()
        }
        Err((status, msg)) => (status, msg).into_response(),
    }
}

async fn build_zip(
    state: Arc<AppState>,
    req: &AnimimgRequest,
    fit: Fit,
) -> Result<Vec<u8>, (StatusCode, String)> {
    // Match the wire-level `start_stream` cap so one expensive POST can't
    // hog memory.
    if req.width == 0
        || req.height == 0
        || req.width > crate::control::fields::MAX_OUTPUT_DIM
        || req.height > crate::control::fields::MAX_OUTPUT_DIM
    {
        return Err((StatusCode::BAD_REQUEST, "width/height out of range".into()));
    }
    let bytes = fetcher::fetch_bytes(&req.source, &state.config.net.user_agent)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("fetch: {e}")))?;

    let method = ResampleMethod::from_str_canon(&state.config.image.method);
    let unsharp = UnsharpParams {
        radius: state.config.image.unsharp.radius,
        amount: state.config.image.unsharp.amount,
        threshold: state.config.image.unsharp.threshold,
    };

    let frames: Vec<(Vec<u8>, f32)> = if dispatch::is_animated(&bytes) {
        let cache = FrameCache::new(0, 1); // per-request, no cross-request share
        let params = AnimatedDispatchParams {
            target_w: req.width,
            target_h: req.height,
            fit,
            method,
            gamma_correct: state.config.image.gamma_correct,
            color_correction: state.config.image.color_correction,
            unsharp,
            source_url: &req.source,
            r#loop: false,
        };
        let seq = dispatch::dispatch(bytes, &params, &cache)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("decode: {e}")))?;
        seq.frames.iter().map(|(b, d)| (b.to_vec(), *d)).collect()
    } else {
        let decoded = decode_bytes(&bytes, &req.source)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("decode: {e}")))?;
        let p = PipelineParams {
            target_w: req.width,
            target_h: req.height,
            fit,
            method,
            gamma_correct: state.config.image.gamma_correct,
            color_correction: state.config.image.color_correction,
            unsharp,
        };
        let rgb = ImagePipeline::run(decoded, &p)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("pipeline: {e}")))?;
        vec![(rgb, 100.0)]
    };

    let frames = rate_limit(frames, req.fps_limit, req.frame_limit as usize);
    if frames.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "no frames extracted".into()));
    }

    encode_zip(&frames, req.width, req.height)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("zip: {e}")))
}

fn rate_limit(
    mut frames: Vec<(Vec<u8>, f32)>,
    fps_limit: Option<f32>,
    frame_limit: usize,
) -> Vec<(Vec<u8>, f32)> {
    if fps_limit.is_none() {
        frames.truncate(frame_limit);
        return frames;
    }
    let min_interval = 1000.0 / fps_limit.unwrap_or(f32::INFINITY);
    let mut acc = 0.0f32;
    let mut kept = 0usize;
    frames.retain(|(_, delay)| {
        if kept >= frame_limit {
            return false;
        }
        acc += *delay;
        if kept == 0 || acc >= min_interval {
            acc = 0.0;
            kept += 1;
            true
        } else {
            false
        }
    });
    frames
}

fn encode_zip(frames: &[(Vec<u8>, f32)], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut buf = Cursor::new(Vec::<u8>::new());
    let mut zip = ZipWriter::new(&mut buf);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut total_ms = 0.0f32;
    let mut yaml_images: Vec<String> = Vec::new();
    let mut yaml_ids: Vec<String> = Vec::new();

    for (i, (rgb888, delay_ms)) in frames.iter().enumerate() {
        let filename = format!("frame_{:03}.png", i + 1);
        let id = format!("frame_{:03}", i + 1);
        total_ms += *delay_ms;

        // Encode the RGB888 buffer → PNG.
        let mut png_bytes: Vec<u8> = Vec::new();
        PngEncoder::new(&mut png_bytes)
            .write_image(rgb888, width, height, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("png encode frame {}: {e}", i + 1))?;

        zip.start_file(&filename, opts).map_err(|e| e.to_string())?;
        zip.write_all(&png_bytes).map_err(|e| e.to_string())?;

        yaml_images.push(format!(
            "  - file: images/{filename}\n    id: {id}\n    type: RGB565"
        ));
        yaml_ids.push(id);
    }

    let yaml = format_yaml(&yaml_images, &yaml_ids, total_ms.round() as u64, frames.len());
    zip.start_file("animimg_config.yaml", opts)
        .map_err(|e| e.to_string())?;
    zip.write_all(yaml.as_bytes()).map_err(|e| e.to_string())?;

    let readme = format_readme(frames.len(), width, height);
    zip.start_file("README.txt", opts).map_err(|e| e.to_string())?;
    zip.write_all(readme.as_bytes()).map_err(|e| e.to_string())?;

    zip.finish().map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}

fn format_yaml(images: &[String], ids: &[String], total_ms: u64, frame_count: usize) -> String {
    let ids_block = ids
        .iter()
        .map(|id| format!("            - {id}"))
        .collect::<Vec<_>>()
        .join("\n");
    let images_block = images.join("\n");
    format!(
        "# ESPHome animimg configuration\n\
         # Generated by media-proxy animimg API\n\
         #\n\
         # Frames: {frame_count}\n\
         # Total animation duration: {total_ms}ms\n\
         \n\
         image:\n{images_block}\n\
         lvgl:\n\
           pages:\n\
             - id: animation_page\n\
               widgets:\n\
                 - animimg:\n\
                     id: my_animation\n\
                     src:\n{ids_block}\n\
                     duration: {total_ms}ms\n\
                     repeat_count: forever\n"
    )
}

fn format_readme(frame_count: usize, width: u32, height: u32) -> String {
    format!(
        "ESPHome LVGL AnimImg Files\n\
         ==========================\n\
         \n\
         This ZIP contains {frame_count} frames at {width}x{height} for the ESPHome LVGL\n\
         animimg widget.\n\
         \n\
         Usage:\n\
           1. Extract the PNG files into your ESPHome project `images/` directory.\n\
           2. Merge `animimg_config.yaml` into your ESPHome YAML.\n\
           3. Tweak the widget position/size and flash the device.\n\
         \n\
         Generated by media-proxy.\n"
    )
}
