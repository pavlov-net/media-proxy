//! `GET /api/internal/homeassistant/{spec}` — Home Assistant lookup shim.
//!
//! Entity/template drawing was intentionally removed in the Rust rewrite.
//! Keep an explicit 501 response for old source URLs.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub async fn homeassistant() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        "Home Assistant entity/template drawing is no longer supported",
    )
        .into_response()
}
