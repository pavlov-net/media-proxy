//! DDP (Distributed Display Protocol) over UDP, plus the collision registry
//! that enforces "one stream per (dest_ip, output_id)".

pub mod packet;
pub mod pixel;
pub mod registry;
pub mod sender;
pub mod spreading;

pub use packet::{
    DDP_HEADER_LEN, DDP_MAX_DATA, DDP_PIXEL_CFG_RGB565_BE, DDP_PIXEL_CFG_RGB565_LE, DDP_PIXEL_CFG_RGB888,
    DdpHeader, PacketEncoder, next_sequence,
};
pub use registry::{DdpKey, DdpRegistry, DdpReservation};
pub use sender::DdpSender;
