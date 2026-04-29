//! DDP packet format.
//!
//! Header layout (big-endian, 10 bytes total):
//! ```text
//! offset 0: flags    u8   0x40 = header present, 0x01 = PUSH (end-of-frame)
//! offset 1: seq      u8   sequence 1..=15 (never 0, never >15)
//! offset 2: cfg      u8   pixel config — 0x0B=RGB888, 0x61=RGB565_BE, 0x62=RGB565_LE
//! offset 3: out_id   u8   destination output/canvas id
//! offset 4: offset   u32  byte offset within frame buffer
//! offset 8: length   u16  payload bytes in this packet
//! ```

use bytes::{BufMut, BytesMut};

use crate::output::sink::PixelFormat;

/// Max payload per DDP packet. 1440 keeps UDP datagrams < 1500B MTU.
pub const DDP_MAX_DATA: usize = 1440;
pub const DDP_HEADER_LEN: usize = 10;

pub const DDP_FLAG_VER1: u8 = 0x40;
pub const DDP_FLAG_PUSH: u8 = 0x01;

pub const DDP_PIXEL_CFG_RGB888: u8 = 0x0B;
pub const DDP_PIXEL_CFG_RGB565_BE: u8 = 0x61;
pub const DDP_PIXEL_CFG_RGB565_LE: u8 = 0x62;

pub fn pixel_cfg_for(format: PixelFormat) -> u8 {
    match format {
        PixelFormat::Rgb888 => DDP_PIXEL_CFG_RGB888,
        PixelFormat::Rgb565Be => DDP_PIXEL_CFG_RGB565_BE,
        PixelFormat::Rgb565Le => DDP_PIXEL_CFG_RGB565_LE,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DdpHeader {
    pub flags: u8,
    pub seq: u8,
    pub cfg: u8,
    pub out_id: u8,
    pub offset: u32,
    pub length: u16,
}

impl DdpHeader {
    pub fn encode_into(&self, buf: &mut BytesMut) {
        buf.put_u8(self.flags);
        buf.put_u8(self.seq);
        buf.put_u8(self.cfg);
        buf.put_u8(self.out_id);
        buf.put_u32(self.offset);
        buf.put_u16(self.length);
    }
}

/// DDP sequence numbers wrap 1..=15 (0 is reserved).
/// `next_sequence(current)` returns the next value to use.
#[inline]
pub fn next_sequence(current: u8) -> u8 {
    (current % 15) + 1
}

/// Stateful packet encoder. Writes each packet into a caller-owned buffer so
/// the sender can reuse a single allocation across all packets of a frame
/// (and across frames).
pub struct PacketEncoder<'a> {
    payload: &'a [u8],
    output_id: u8,
    cfg: u8,
    seq: u8,
    offset: usize,
}

impl<'a> PacketEncoder<'a> {
    pub fn new(payload: &'a [u8], output_id: u8, starting_seq: u8, format: PixelFormat) -> Self {
        Self {
            payload,
            output_id,
            cfg: pixel_cfg_for(format),
            seq: starting_seq,
            offset: 0,
        }
    }

    /// Sequence value the next emitted packet would carry. After the last
    /// packet has been emitted, this is the sequence the *next* frame should
    /// start with.
    pub fn current_seq(&self) -> u8 {
        self.seq
    }

    /// Write the next packet into `buf` (cleared first). Returns `Some(len)`
    /// where `len` is the bytes written to `buf`, or `None` when the payload
    /// has been fully drained. `buf` must have capacity
    /// `DDP_HEADER_LEN + DDP_MAX_DATA`.
    pub fn encode_next(&mut self, buf: &mut BytesMut) -> Option<usize> {
        if self.offset >= self.payload.len() {
            return None;
        }
        let end = (self.offset + DDP_MAX_DATA).min(self.payload.len());
        let chunk = &self.payload[self.offset..end];
        let is_last = end >= self.payload.len();
        let flags = DDP_FLAG_VER1 | if is_last { DDP_FLAG_PUSH } else { 0 };

        buf.clear();
        DdpHeader {
            flags,
            seq: self.seq,
            cfg: self.cfg,
            out_id: self.output_id,
            offset: self.offset as u32,
            length: chunk.len() as u16,
        }
        .encode_into(buf);
        buf.extend_from_slice(chunk);

        self.seq = next_sequence(self.seq);
        self.offset = end;
        Some(buf.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_wraps_1_through_15() {
        let mut s: u8 = 1;
        for expected in [2u8, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 1, 2] {
            s = next_sequence(s);
            assert_eq!(s, expected);
        }
    }

    #[test]
    fn sequence_never_zero() {
        for s in 1u8..=15 {
            assert_ne!(next_sequence(s), 0);
        }
    }

    #[test]
    fn header_encodes_to_10_bytes() {
        let h = DdpHeader {
            flags: 0x41,
            seq: 1,
            cfg: DDP_PIXEL_CFG_RGB888,
            out_id: 7,
            offset: 0,
            length: 100,
        };
        let mut buf = BytesMut::new();
        h.encode_into(&mut buf);
        assert_eq!(buf.len(), DDP_HEADER_LEN);
        // bytes 0..=3: flags, seq, cfg, out_id
        assert_eq!(&buf[..4], &[0x41, 1, DDP_PIXEL_CFG_RGB888, 7]);
        // bytes 4..=7: offset=0 big-endian u32
        assert_eq!(&buf[4..8], &[0, 0, 0, 0]);
        // bytes 8..=9: length=100 big-endian u16
        assert_eq!(&buf[8..10], &100u16.to_be_bytes());
    }

    fn collect_packets(payload: &[u8], starting_seq: u8) -> Vec<Vec<u8>> {
        let mut enc = PacketEncoder::new(payload, 1, starting_seq, PixelFormat::Rgb888);
        let mut buf = BytesMut::with_capacity(DDP_HEADER_LEN + DDP_MAX_DATA);
        let mut out = Vec::new();
        while let Some(len) = enc.encode_next(&mut buf) {
            out.push(buf[..len].to_vec());
        }
        out
    }

    #[test]
    fn single_packet_has_push_flag() {
        let payload = vec![0u8; 100];
        let packets = collect_packets(&payload, 1);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0][0] & DDP_FLAG_PUSH, DDP_FLAG_PUSH);
    }

    #[test]
    fn multi_packet_only_last_has_push() {
        // 1441 bytes → 2 packets (1440 + 1)
        let payload = vec![0u8; DDP_MAX_DATA + 1];
        let packets = collect_packets(&payload, 1);
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0][0] & DDP_FLAG_PUSH, 0);
        assert_eq!(packets[1][0] & DDP_FLAG_PUSH, DDP_FLAG_PUSH);
    }

    #[test]
    fn returned_next_seq_matches_wrap() {
        let payload = vec![0u8; 100];
        let mut enc = PacketEncoder::new(&payload, 1, 15, PixelFormat::Rgb888);
        let mut buf = BytesMut::with_capacity(DDP_HEADER_LEN + DDP_MAX_DATA);
        let _ = enc.encode_next(&mut buf).unwrap();
        assert_eq!(enc.current_seq(), 1);
    }
}
