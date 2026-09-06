//! Per-connection control session. Owns active stream handles so the cleanup
//! path can cancel them when the socket drops.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;

use crate::output::sink::StreamId;
use crate::stream::handle::StreamHandle;

/// Client-addressed stream identity, independent of the internal `StreamId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientKey {
    Ddp { dest: IpAddr, output_id: u8 },
}

#[derive(Debug)]
pub struct Session {
    pub client_id: String,
    pub client_ip: IpAddr,
    pub server_host: String,
    pub device_id: Mutex<Option<String>>,
    pub streams: Mutex<HashMap<ClientKey, StreamHandle>>,
    pub stream_ids: Mutex<HashMap<StreamId, ClientKey>>,
}

impl Session {
    pub fn new(client_ip: IpAddr, server_host: String) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let client_id = format!("ws-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        Self {
            client_id,
            client_ip,
            server_host,
            device_id: Mutex::new(None),
            streams: Mutex::new(HashMap::new()),
            stream_ids: Mutex::new(HashMap::new()),
        }
    }
}
