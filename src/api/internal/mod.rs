//! `internal:` URL scheme endpoints.
//!
//! `internal:placeholder/...` → this server
//! `internal:ha/...` → Home Assistant (delegated to `media-proxy-addon`)

pub mod homeassistant;
pub mod placeholder;

pub use homeassistant::homeassistant;
pub use placeholder::placeholder;
