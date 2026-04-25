//! `GET /api/internal/homeassistant/{spec}` — Home Assistant lookup shim.
//!
//! Returns 501 here; the `media-proxy-addon` owns this endpoint and the
//! response signals it to route around us.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub async fn homeassistant() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        "HA endpoint served by media-proxy-addon",
    )
        .into_response()
}
