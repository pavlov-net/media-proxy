//! Handler dispatch: client messages → orchestrator actions.

use std::sync::Arc;

use tracing::{info, warn};

use crate::Config;
use crate::control::fields::{StartStream, StopStream, StreamFields, UpdateStream};
use crate::control::protocol::{ClientMsg, ErrorMsg, ServerMsg};
use crate::control::session::{ClientKey, Session};
use crate::error::ControlError;
use crate::stream::orchestrator::Orchestrator;

pub struct Handler {
    pub config: Arc<Config>,
    pub orch: Arc<Orchestrator>,
}

impl Handler {
    pub async fn dispatch(&self, session: &Session, msg: ClientMsg) -> Result<ServerMsg, ControlError> {
        match msg {
            ClientMsg::Hello { device_id } => {
                let device = device_id.unwrap_or_else(|| "unknown".to_string());
                *session.device_id.lock() = Some(device.clone());
                info!(client = %session.client_ip, device = %device, "hello");
                Ok(ServerMsg::HelloAck {
                    server_version: concat!("media-proxy/", env!("CARGO_PKG_VERSION")).into(),
                })
            }
            ClientMsg::StartStream(req) => self.handle_start(session, req).await,
            ClientMsg::StopStream(req) => self.handle_stop(session, req).await,
            ClientMsg::Update(req) => self.handle_update(session, req).await,
            ClientMsg::Ping { t } => Ok(ServerMsg::Pong { t }),
        }
    }

    async fn handle_start(&self, session: &Session, req: StartStream) -> Result<ServerMsg, ControlError> {
        let fields = StreamFields::from_start(&req, session.client_ip, &session.server_host, &self.config)?;
        let out_id = fields.output_id;
        info!(
            client = %session.client_ip,
            out = out_id,
            size = %format!("{}x{}", fields.width, fields.height),
            src = %fields.source,
            "start_stream"
        );
        // Snapshot the `applied` view after resolving `hw=auto` → concrete backend.
        let applied = self.resolve_applied(&fields);

        let key = ClientKey::Ddp {
            dest: fields.ddp_host,
            output_id: crate::control::fields::output_id_byte(out_id),
        };

        // Orchestrator spawns the stream task; returns a handle that the
        // session owns for cancellation.
        let handle = self
            .orch
            .spawn_stream(fields)
            .await
            .map_err(|e| ControlError::Protocol(e.to_string()))?;

        // If an older stream was on this key, the DDP registry already
        // cancelled it; we just need to drop any handle we were tracking.
        let old_handle = session.streams.lock().insert(key, handle.clone());
        if let Some(old) = old_handle {
            old.cancel();
            session.stream_ids.lock().retain(|_, v| *v != key);
        }
        session.stream_ids.lock().insert(handle.stream_id(), key);

        Ok(ServerMsg::Ack {
            out: out_id,
            applied: Some(applied),
        })
    }

    async fn handle_stop(&self, session: &Session, req: StopStream) -> Result<ServerMsg, ControlError> {
        let out_id = req.out;
        let dest = match req.ddp_host.as_deref() {
            Some(s) => s
                .parse()
                .map_err(|e| ControlError::BadRequest(format!("ddp_host parse: {e}")))?,
            None => session.client_ip,
        };
        let key = ClientKey::Ddp {
            dest,
            output_id: crate::control::fields::output_id_byte(out_id),
        };
        let handle = session.streams.lock().remove(&key);
        if let Some(h) = handle {
            h.cancel();
        } else {
            warn!(out = out_id, "stop_stream for unknown output");
        }
        Ok(ServerMsg::Ack {
            out: out_id,
            applied: None,
        })
    }

    async fn handle_update(&self, session: &Session, req: UpdateStream) -> Result<ServerMsg, ControlError> {
        // An update is a stream restart that inherits unset fields from the
        // prior stream for this (dest_ip, output_id). Rejecting uninitialized
        // updates when there's no active stream matches Python's behavior.
        let out_id = req.out;
        let dest = match req.ddp_host.as_deref() {
            Some(s) => s
                .parse()
                .map_err(|e| ControlError::BadRequest(format!("ddp_host parse: {e}")))?,
            None => session.client_ip,
        };
        let key = ClientKey::Ddp {
            dest,
            output_id: crate::control::fields::output_id_byte(out_id),
        };

        let prior_fields = {
            let streams = session.streams.lock();
            let handle = streams.get(&key).ok_or(ControlError::UnknownOutput(out_id))?;
            handle.fields().clone()
        };

        // Overlay update-supplied fields onto the prior stream's view.
        let merged = crate::control::fields::merge_update(&prior_fields, &req, &session.server_host)?;
        self.start_with_fields(session, merged).await
    }

    async fn start_with_fields(
        &self,
        session: &Session,
        fields: StreamFields,
    ) -> Result<ServerMsg, ControlError> {
        let out_id = fields.output_id;
        info!(
            client = %session.client_ip,
            out = out_id,
            size = %format!("{}x{}", fields.width, fields.height),
            src = %fields.source,
            "update_stream"
        );
        let applied = self.resolve_applied(&fields);
        let key = ClientKey::Ddp {
            dest: fields.ddp_host,
            output_id: crate::control::fields::output_id_byte(out_id),
        };

        let handle = self
            .orch
            .spawn_stream(fields)
            .await
            .map_err(|e| ControlError::Protocol(e.to_string()))?;

        let old_handle = session.streams.lock().insert(key, handle.clone());
        if let Some(old) = old_handle {
            old.cancel();
            session.stream_ids.lock().retain(|_, v| *v != key);
        }
        session.stream_ids.lock().insert(handle.stream_id(), key);

        Ok(ServerMsg::Ack {
            out: out_id,
            applied: Some(applied),
        })
    }

    /// Build the `applied` view returned in `ack`. Resolves `hw=auto` to the
    /// concrete backend so clients see the actual selection.
    fn resolve_applied(&self, fields: &StreamFields) -> crate::control::fields::AppliedParams {
        let mut applied = fields.to_applied();
        if matches!(fields.hw, crate::control::fields::HwPref::Auto)
            && let Some(backend) = crate::video::hwaccel::pick_for(fields.hw)
        {
            applied.hw = match backend {
                crate::platform::HwBackend::Cuda => crate::control::fields::HwPref::Cuda,
                crate::platform::HwBackend::Qsv => crate::control::fields::HwPref::Qsv,
                crate::platform::HwBackend::Vaapi => crate::control::fields::HwPref::Vaapi,
                crate::platform::HwBackend::D3d11va => crate::control::fields::HwPref::D3d11va,
                crate::platform::HwBackend::Videotoolbox => crate::control::fields::HwPref::Videotoolbox,
            };
        }
        applied
    }

    /// Convert any `ControlError` into the wire-level error frame.
    pub fn error_response(e: &ControlError) -> ServerMsg {
        ServerMsg::Error(ErrorMsg {
            code: e.code().into(),
            message: e.to_string(),
        })
    }
}
