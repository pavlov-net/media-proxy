//! Client control plane: WebSocket protocol, session tracking, field
//! validation, handler dispatch.

pub mod fields;
pub mod handler;
pub mod protocol;
pub mod session;
pub mod ws;

pub use fields::{AppliedParams, StartStream, StopStream, StreamFields, UpdateStream};
pub use protocol::{ClientMsg, ErrorMsg, ServerMsg};
pub use session::Session;
