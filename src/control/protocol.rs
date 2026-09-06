//! JSON control messages and serialization.
//! Message shapes: `hello`, `start_stream`, `stop_stream`, `update`, `ping`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::control::fields::{AppliedParams, StartStream, StopStream, UpdateStream};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        #[serde(default)]
        device_id: Option<String>,
    },
    StartStream(StartStream),
    StopStream(StopStream),
    Update(UpdateStream),
    Ping {
        t: Option<Value>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    HelloAck {
        server_version: String,
    },
    Ack {
        out: i32,
        #[serde(skip_serializing_if = "Option::is_none")]
        applied: Option<AppliedParams>,
    },
    Pong {
        t: Option<Value>,
    },
    Error(ErrorMsg),
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorMsg {
    pub code: String,
    pub message: String,
}

/// Parses a control message, ignoring unknown fields.
pub fn parse_client_msg(raw: &str) -> serde_json::Result<ClientMsg> {
    serde_json::from_str(raw)
}

pub fn serialize_server_msg(msg: &ServerMsg) -> String {
    serde_json::to_string(msg).unwrap_or_else(|e| {
        // Keep a wire error response available if serialization fails.
        format!(r#"{{"type":"error","code":"server_error","message":"{}"}}"#, e)
    })
}

/// JSON object type for untyped protocol fields.
#[allow(dead_code)]
pub type RawMap = serde_json::Map<String, Value>;
