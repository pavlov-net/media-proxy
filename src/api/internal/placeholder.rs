//! `GET /api/internal/placeholder/{spec}` — generate a PNG with text.
//!
//! URL patterns (extension required):
//!   /placeholder/64x64.png
//!   /placeholder/600x400/orange/white.png
//!   /placeholder/600x400/ff0000.png
//!   /placeholder/800.png?text=Hello+World

use std::io::Cursor;

use axum::extract::{Path, Query};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use image::ImageEncoder;
use image::codecs::png::PngEncoder;
use serde::Deserialize;

use crate::render::color;
use crate::render::render_bbcode_text;

#[derive(Deserialize)]
pub struct PlaceholderQuery {
    text: Option<String>,
}

pub async fn placeholder(Path(spec): Path<String>, Query(q): Query<PlaceholderQuery>) -> Response {
    let Some(spec) = spec.strip_suffix(".png") else {
        return (StatusCode::BAD_REQUEST, "Extension required. Use .png").into_response();
    };
    let parts: Vec<&str> = spec.split('/').collect();
    let Some(first) = parts.first() else {
        return (StatusCode::BAD_REQUEST, "Missing dimensions").into_response();
    };
    let (width, height) = match parse_dims(first) {
        Some(d) => d,
        None => return (StatusCode::BAD_REQUEST, "Invalid dimensions format").into_response(),
    };
    if !(10..=4096).contains(&width) || !(10..=4096).contains(&height) {
        return (StatusCode::BAD_REQUEST, "Dimensions must be 10-4096px").into_response();
    }

    let bg = parts
        .get(1)
        .and_then(|s| color::parse(s))
        .unwrap_or([204, 204, 204]);
    let fg = parts
        .get(2)
        .and_then(|s| color::parse(s))
        .unwrap_or_else(|| color::auto_contrast(bg));
    let text = q
        .text
        .clone()
        .unwrap_or_else(|| format!("{width}x{height}"))
        .replace("\\n", "\n");

    let rendered = match render_bbcode_text(&text, width, height, fg, bg) {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("render: {e}")).into_response(),
    };

    let mut buf = Cursor::new(Vec::<u8>::new());
    if PngEncoder::new(&mut buf)
        .write_image(
            &rendered.rgb888,
            rendered.width,
            rendered.height,
            image::ExtendedColorType::Rgb8,
        )
        .is_err()
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, "PNG encode failed").into_response();
    }

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000"),
    );
    (headers, buf.into_inner()).into_response()
}

fn parse_dims(s: &str) -> Option<(u32, u32)> {
    if let Some((w, h)) = s.split_once('x') {
        Some((w.parse().ok()?, h.parse().ok()?))
    } else {
        let n: u32 = s.parse().ok()?;
        Some((n, n))
    }
}
