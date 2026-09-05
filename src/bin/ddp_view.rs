//! Terminal DDP receiver and optional WebSocket client. Frames use upper-half
//! block glyphs, displaying two pixel rows per terminal row.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::json;
use tokio::net::UdpSocket;
use tokio::signal;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const DDP_HEADER_LEN: usize = 10;
const DDP_FLAG_PUSH: u8 = 0x01;
const PIXEL_RGB888: u8 = 0x0B;
const PIXEL_RGB565_BE: u8 = 0x61;
const PIXEL_RGB565_LE: u8 = 0x62;

/// Limits rendering frequency to avoid terminal saturation.
const RENDER_INTERVAL: Duration = Duration::from_millis(33); // ~30 fps

#[derive(Debug, Parser)]
#[command(name = "ddp-view", about = "Terminal DDP receiver / media-proxy test client")]
struct Cli {
    /// UDP port to listen on for DDP packets.
    #[arg(long, default_value_t = 4048)]
    listen_port: u16,

    /// Bind address for the UDP listener.
    #[arg(long, default_value = "0.0.0.0")]
    listen_host: IpAddr,

    /// Frame width in pixels; must match the received stream.
    #[arg(long, default_value_t = 64)]
    width: u32,

    /// Frame height in pixels; must match the received stream.
    #[arg(long, default_value_t = 64)]
    height: u32,

    /// Output ID to render.
    #[arg(long, default_value_t = 1)]
    out: u8,

    /// WebSocket URL for controlling a media-proxy stream into this viewer.
    #[arg(long)]
    connect: Option<String>,

    /// Source URL/path to stream (only used with `--connect`).
    #[arg(long, requires = "connect")]
    src: Option<String>,

    /// Destination IP override; otherwise media-proxy uses the WebSocket client IP.
    #[arg(long, requires = "connect")]
    ddp_host: Option<String>,

    /// Fit mode for the controlled stream.
    #[arg(long, default_value = "auto", requires = "connect")]
    fit: String,

    /// Pixel format for the controlled stream.
    #[arg(long, default_value = "rgb888", requires = "connect")]
    fmt: String,

    /// Plays the controlled stream once.
    #[arg(long, requires = "connect")]
    no_loop: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let sock = UdpSocket::bind(SocketAddr::new(cli.listen_host, cli.listen_port)).await?;
    eprintln!(
        "[ddp-view] listening on {}:{} for out={} ({}x{})",
        cli.listen_host, cli.listen_port, cli.out, cli.width, cli.height
    );

    let ws_task = if let Some(url) = cli.connect.clone() {
        let src = cli
            .src
            .clone()
            .ok_or_else(|| anyhow::anyhow!("--src is required with --connect"))?;
        let req = StartStreamReq {
            url,
            out: cli.out,
            width: cli.width,
            height: cli.height,
            src,
            ddp_port: cli.listen_port,
            ddp_host: cli.ddp_host.clone(),
            fit: cli.fit.clone(),
            fmt: cli.fmt.clone(),
            r#loop: !cli.no_loop,
        };
        Some(tokio::spawn(run_ws_controller(req)))
    } else {
        None
    };

    let frames: Arc<Mutex<HashMap<u8, FrameBuf>>> = Arc::new(Mutex::new(HashMap::new()));

    let recv_loop = receive_and_render(sock, frames.clone(), cli.out, cli.width, cli.height);
    tokio::select! {
        r = recv_loop => r?,
        _ = signal::ctrl_c() => {
            eprintln!("\n[ddp-view] interrupt — shutting down");
        }
    }

    if let Some(handle) = ws_task {
        handle.abort();
    }

    // Restore terminal state on exit.
    print!("\x1b[0m\x1b[?25h\x1b[2J\x1b[H");
    let _ = std::io::stdout().flush();
    Ok(())
}

struct FrameBuf {
    pixel_cfg: u8,
    /// Payload storage in the sender's pixel format.
    data: Vec<u8>,
    /// Rendering timestamp for rate limiting.
    last_render: Instant,
}

impl FrameBuf {
    fn new(pixel_cfg: u8, capacity: usize) -> Self {
        Self {
            pixel_cfg,
            data: vec![0u8; capacity],
            last_render: Instant::now() - RENDER_INTERVAL,
        }
    }
}

async fn receive_and_render(
    sock: UdpSocket,
    frames: Arc<Mutex<HashMap<u8, FrameBuf>>>,
    filter_out: u8,
    w: u32,
    h: u32,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 1500];
    let mut frames_drawn: u64 = 0;
    let mut bytes_seen: u64 = 0;
    let mut last_drawn_dims: Option<(u32, u32)> = None;

    loop {
        let n = sock.recv(&mut buf).await?;
        if n < DDP_HEADER_LEN {
            continue;
        }
        let flags = buf[0];
        // Packet sequence is informational; assembly uses byte offsets.
        let pixel_cfg = buf[2];
        let out_id = buf[3];
        let offset = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        let length = u16::from_be_bytes([buf[8], buf[9]]) as usize;
        let push = (flags & DDP_FLAG_PUSH) != 0;

        if out_id != filter_out {
            continue;
        }

        let payload_end = (DDP_HEADER_LEN + length).min(n);
        let payload = &buf[DDP_HEADER_LEN..payload_end];
        bytes_seen += payload.len() as u64;

        let frame_size = match pixel_cfg {
            PIXEL_RGB888 => (w * h * 3) as usize,
            PIXEL_RGB565_BE | PIXEL_RGB565_LE => (w * h * 2) as usize,
            other => {
                eprintln!("[ddp-view] unknown pixel cfg 0x{other:02x}, dropping packet");
                continue;
            }
        };

        let render_now = {
            let mut map = frames.lock();
            let fb = map
                .entry(out_id)
                .and_modify(|fb| {
                    if fb.pixel_cfg != pixel_cfg || fb.data.len() != frame_size {
                        *fb = FrameBuf::new(pixel_cfg, frame_size);
                    }
                })
                .or_insert_with(|| FrameBuf::new(pixel_cfg, frame_size));

            if offset + payload.len() <= fb.data.len() {
                fb.data[offset..offset + payload.len()].copy_from_slice(payload);
            }

            push && fb.last_render.elapsed() >= RENDER_INTERVAL
        };

        if render_now {
            let snapshot = {
                let mut map = frames.lock();
                let Some(fb) = map.get_mut(&out_id) else { continue };
                fb.last_render = Instant::now();
                (fb.pixel_cfg, fb.data.clone())
            };
            let rgb = decode_to_rgb888(&snapshot.1, snapshot.0);
            // Clear on first render or dimension changes; overwrite otherwise to avoid flicker.
            let clear = last_drawn_dims != Some((w, h));
            render(&rgb, w, h, out_id, frames_drawn, bytes_seen, clear);
            last_drawn_dims = Some((w, h));
            frames_drawn += 1;
        }
    }
}

fn decode_to_rgb888(data: &[u8], pixel_cfg: u8) -> Vec<u8> {
    match pixel_cfg {
        PIXEL_RGB888 => data.to_vec(),
        PIXEL_RGB565_BE => decode_565(data, true),
        PIXEL_RGB565_LE => decode_565(data, false),
        _ => Vec::new(),
    }
}

fn decode_565(data: &[u8], be: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2 * 3);
    for chunk in data.as_chunks::<2>().0 {
        let v = if be {
            u16::from_be_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_le_bytes([chunk[0], chunk[1]])
        };
        let r5 = (v >> 11) & 0x1F;
        let g6 = (v >> 5) & 0x3F;
        let b5 = v & 0x1F;
        // Replicate high bits into low bits when expanding RGB565 channels.
        let r = ((r5 << 3) | (r5 >> 2)) as u8;
        let g = ((g6 << 2) | (g6 >> 4)) as u8;
        let b = ((b5 << 3) | (b5 >> 2)) as u8;
        out.extend_from_slice(&[r, g, b]);
    }
    out
}

fn render(rgb: &[u8], w: u32, h: u32, out_id: u8, frames_drawn: u64, bytes_seen: u64, clear: bool) {
    let mut stdout = std::io::stdout().lock();
    if clear {
        let _ = write!(stdout, "\x1b[2J\x1b[?25l\x1b[H");
    } else {
        // Overwrite in place and erase line endings so shorter status text leaves no residue.
        let _ = write!(stdout, "\x1b[H");
    }
    let _ = writeln!(
        stdout,
        "ddp out={out_id} {w}x{h}  frames={frames_drawn}  bytes={bytes_seen}\x1b[0m\x1b[K"
    );

    let pixel = |x: u32, y: u32| -> [u8; 3] {
        if y >= h || x >= w {
            return [0, 0, 0];
        }
        let i = ((y * w + x) * 3) as usize;
        if i + 3 <= rgb.len() {
            [rgb[i], rgb[i + 1], rgb[i + 2]]
        } else {
            [0, 0, 0]
        }
    };

    for y in (0..h).step_by(2) {
        for x in 0..w {
            let [tr, tg, tb] = pixel(x, y);
            let [br, bg, bb] = pixel(x, y + 1);
            // Each block glyph combines a foreground upper pixel and background lower pixel.
            let _ = write!(stdout, "\x1b[38;2;{tr};{tg};{tb};48;2;{br};{bg};{bb}m\u{2580}");
        }
        let _ = writeln!(stdout, "\x1b[0m\x1b[K");
    }
    let _ = stdout.flush();
}

#[derive(Debug, Clone)]
struct StartStreamReq {
    url: String,
    out: u8,
    width: u32,
    height: u32,
    src: String,
    ddp_port: u16,
    ddp_host: Option<String>,
    fit: String,
    fmt: String,
    r#loop: bool,
}

async fn run_ws_controller(req: StartStreamReq) -> anyhow::Result<()> {
    eprintln!("[ddp-view] connecting to {}", req.url);
    let (mut ws, _resp) = connect_async(&req.url).await?;

    let hello = json!({"type": "hello", "device_id": "ddp-view"}).to_string();
    ws.send(Message::Text(hello.into())).await?;

    // Omitted optional fields retain the server's defaults.
    let mut start = serde_json::Map::new();
    start.insert("type".into(), json!("start_stream"));
    start.insert("out".into(), json!(req.out));
    start.insert("w".into(), json!(req.width));
    start.insert("h".into(), json!(req.height));
    start.insert("src".into(), json!(req.src));
    start.insert("ddp_port".into(), json!(req.ddp_port));
    if let Some(host) = req.ddp_host {
        start.insert("ddp_host".into(), json!(host));
    }
    start.insert("fit".into(), json!(req.fit));
    start.insert("fmt".into(), json!(req.fmt));
    start.insert("loop".into(), json!(req.r#loop));
    let payload = serde_json::Value::Object(start).to_string();
    ws.send(Message::Text(payload.into())).await?;

    // Reading the socket lets tungstenite send automatic Pong replies.
    while let Some(msg) = ws.next().await {
        match msg {
            Ok(Message::Text(t)) => eprintln!("[ws] {t}"),
            Ok(Message::Close(frame)) => {
                eprintln!("[ws] close: {frame:?}");
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[ws] error: {e}");
                break;
            }
        }
    }
    Ok(())
}
