//! Home Assistant drawing URLs return 501; entity/template drawing is unsupported.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub async fn homeassistant() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        "Home Assistant entity/template drawing is unsupported",
    )
        .into_response()
}
