# Media Proxy

Stream videos, GIFs, still images, and YouTube content to tiny LED/LCD displays (e.g., ESPHome devices using DDP) with smart resizing, optional color-range expansion, and packet pacing. Exposes a WebSocket control API and pushes pixel data over UDP using the DDP format.

Available as a Home Assistant add-on or standalone Rust binary.

---

## What you get

- Unified HTTP/WebSocket server (default port `8788`) with WebSocket control API and REST endpoints.
- Video decoding via subprocess `ffmpeg` with hardware acceleration (CUDA, QSV, VAAPI, VideoToolbox, D3D11VA).
- Smart fit modes (`cover`, `pad`, `auto`), optional TV→PC range expansion, and automatic black-bar crop.
- Static + animated image pipeline (PNG, JPEG, GIF, APNG, WebP) with disposal/blend handling and an LRU frame cache.
- UDP DDP sender with optional packet spreading, pacing, and still-frame redundancy.
- Companion `ddp-view` binary — a terminal DDP receiver that renders 64×64 frames using half-block characters, useful for end-to-end testing without an LED panel.

---

## Requirements

- Rust 1.98.1+ (edition 2024) — only needed to build from source
- `ffmpeg` on `PATH` for video sources
- Network access to your displays (UDP, typically port 4048)

YouTube resolution + Deno EJS solver live in the companion [`media-proxy-addon`](https://github.com/stuartparmenter/media-proxy-addon); the core does not bundle yt-dlp.

---

## Installation

### Home Assistant Add-on

1. In Home Assistant, open **Settings → Add-ons → Add-on Store**.
2. Click **⋮ → Repositories** and add:
   `https://github.com/stuartparmenter/homeassistant-addons`
3. Find **Media Proxy** in the list and click **Install**.
4. After install, open the add-on:
   - Optionally enable **Start on boot** and **Watchdog**.
   - Add configuration (see below).
   - Click **Start**, then check the **Log** tab to verify it's running.

### Standalone Binary

```bash
git clone https://github.com/stuartparmenter/media-proxy.git
cd media-proxy
cargo build --release
./target/release/media-proxy --host 0.0.0.0 --port 8788
```

CLI flags:

```
--host <HOST>            [default: 0.0.0.0]
--port <PORT>            [default: 8788]
--config <CONFIG>        Path to YAML/TOML/JSON config file (optional)
--log-level <LEVEL>      debug | info | warning | error | critical
```

Configuration also accepts `MEDIA_PROXY__SECTION__KEY=value` env-var overrides
(e.g. `MEDIA_PROXY__IMAGE__GAMMA_CORRECT=true`).

---

## Configuration

By default the server runs on `host: 0.0.0.0`, `port: 8788`.

You can optionally provide a configuration file using the `--config` argument. Without a config file, the application uses built-in defaults.

### Example config file

```yaml
hw:
  prefer: auto

video:
  expand_mode: 2        # 0=never, 1=auto(limited->full), 2=force
  fit: auto             # cover | pad | auto
  autocrop:
    enabled: false      # Disabled by default for safety
    probe_frames: 24    # More frames for stability
    luma_thresh: 16     # Lower threshold for conservative detection
    max_bar_ratio: 0.15 # More conservative cropping limit
    min_bar_px: 2

playback:
  loop: true

youtube:
  60fps: true           # Try 60fps at 720p first, then fall back to resolution-optimized selection

image:
  method: lanczos       # lanczos | bicubic | bilinear | box | nearest | auto
  gamma_correct: false
  color_correction: true
  unsharp:
    amount: 0.0
    radius: 0.6
    threshold: 2
  frame_cache_mb: 32    # Max memory for cached frames (0 = disabled)
  frame_cache_min_frames: 5  # Only cache if animation has >= N frames

log:
  level: info
  metrics: true
  rate_ms: 5000
  send_ms: false

net:
  win_timer_res: true
  spread_packets: true
  spread_max_fps: 60
  spread_min_ms: 3.0
  spread_max_sleeps: 0

playback_still:
  redundancy: 3  # Send each packet this many times for reliability
```

---

## Video Processing Options

### Fit Modes

- **`auto` (recommended)**: Smart mode that avoids unnecessary processing
  - When source and target aspect ratios match: direct scaling with no padding/cropping
  - When aspect ratios differ: falls back to `pad` behavior (preserves all content)
  - Optimal for most use cases - maximum efficiency with no content loss

- **`pad`**: Always preserves all content by adding black bars when needed
  - Guarantees no content is cropped
  - May add unnecessary padding even when aspect ratios match

- **`cover`**: Fills display completely but may crop content
  - Scales to fill the display and crops excess content
  - Use only when you're okay with potentially losing parts of the image/video

### Automatic Black Bar Cropping

Autocrop automatically detects and removes letterbox/pillarbox bars from videos. It's disabled by default to avoid cropping legitimate dark content.

```yaml
video:
  autocrop:
    enabled: false      # Enable if needed for letterboxed content
    probe_frames: 24    # Samples more frames for stability
    luma_thresh: 16     # Conservative threshold to avoid dark content
    max_bar_ratio: 0.15 # Won't crop more than 15% from any edge
```

**How it works:**
- Analyzes the first 24 frames to detect consistent black borders
- Uses conservative settings to minimize false positives
- Once detected, applies the same crop to the entire stream

**Considerations:**
- May crop dark scenes or fade-to-black sequences if they occur early in the video
- Best suited for content with consistent letterboxing throughout

---

## YouTube Optimization

Media Proxy intelligently selects YouTube formats based on your display size and hardware acceleration:

**Resolution Matching:** Automatically selects the most appropriate resolution for your display:
- 64×64 displays use 144p → 240p → 360p streams
- 480×480 displays use 480p → 360p → 720p streams
- Reduces bandwidth usage while maintaining visual quality

**60fps Content:** With `youtube.60fps: true` (default), prioritizes smooth motion:
- Attempts 60fps at 720p first, regardless of display size (YouTube only offers 60fps at 720p+)
- Falls back to resolution-matched formats if 60fps unavailable
- Set to `false` to prioritize bandwidth/CPU efficiency over framerate

**Hardware Acceleration:** Codec selection optimized for your acceleration method:
- **VAAPI:** AV1 → VP9 → H.265 → H.264 (modern codec preference)
- **Quick Sync:** H.265 → H.264 → AV1 (optimized for Intel's HEVC support)
- **CUDA:** AV1 → H.265 → H.264 (RTX 30+ series supports AV1 decode)
- **CPU fallback:** H.264 → VP9 → H.265 (efficiency-focused)

Enable `log.level: debug` to see format selection details.

---

## WebSocket Control API

Media Proxy provides a WebSocket control API on `/control` (default port `:8788`) for managing video/image streams to devices.

### Connection & Handshake

1. **Connect** to `ws://localhost:8788/control`
2. **Send handshake** message:
```json
{
  "type": "hello",
  "device_id": "my_device_123"
}
```
3. **Receive acknowledgment**:
```json
{
  "type": "hello_ack",
  "server_version": "media-proxy/1.0"
}
```

### Control Messages

#### Start Stream
```json
{
  "type": "start_stream",
  "out": 5,
  "w": 64,
  "h": 64,
  "src": "https://example.com/video.mp4",
  "ddp_port": 4048,
  "fit": "auto",
  "loop": true,
  "hw": "auto"
}
```

**Required parameters:**
- `out`: Output ID (integer ≥ 1)
- `w`: Display width in pixels (integer > 0)
- `h`: Display height in pixels (integer > 0)
- `src`: Media source (file path, HTTP URL, or YouTube URL)

**Optional parameters:**
- `ddp_host`: Target hostname/IP for DDP packets (default: client's IP address)
- `ddp_port`: DDP output port (default: 4048)
- `fit`: Resize mode - `cover`, `pad`, or `auto` (default: `auto`)
  - `auto`: Smart fit - scales directly when aspect ratios match, adds padding when they don't (recommended)
  - `pad`: Always scales to fit and adds black bars if needed (never crops content)
  - `cover`: Scales to fill and crops excess content (may lose parts of the image/video)
- `loop`: Loop media playback (boolean, default from config)
- `hw`: Hardware acceleration - `auto`, `none`, `cuda`, `qsv`, `vaapi`, `videotoolbox`, `d3d11va`
- `fmt`: Pixel format - `rgb888`, `rgb565le`, `rgb565be` (default: `rgb888`)
- `expand`: Color range expansion - `0` (never), `1` (auto), `2` (force)
- `pace`: Frame pacing frequency in Hz (default: 0)
- `ema`: EMA filter alpha (0.0-1.0, default: 0.0)

#### Stop Stream
```json
{
  "type": "stop_stream",
  "out": 5
}
```

#### Update Stream
Updates an existing stream with new parameters (stops and restarts internally):
```json
{
  "type": "update",
  "out": 5,
  "src": "https://example.com/new_video.mp4",
  "loop": false
}
```

### Error Responses
```json
{
  "type": "error",
  "code": "bad_request",
  "message": "start requires Display width (missing: w)"
}
```

**Error codes:**
- `proto`: Protocol violation (invalid handshake, bad JSON)
- `bad_type`: Unknown message type
- `bad_request`: Missing/invalid parameters
- `server_error`: Internal server error

### Example JavaScript Client
```javascript
const ws = new WebSocket('ws://localhost:8788/control');

ws.onopen = () => {
  // Handshake
  ws.send(JSON.stringify({
    type: 'hello',
    device_id: 'browser_client'
  }));
};

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.type === 'hello_ack') {
    // Start streaming
    ws.send(JSON.stringify({
      type: 'start_stream',
      out: 5,
      w: 64,
      h: 64,
      src: 'https://media.giphy.com/media/3o6Zt481isNVuQI1l6/giphy.gif'
    }));
  }
};
```

---

## HTTP REST API

Media Proxy provides HTTP endpoints for health checks and placeholder image generation.

### GET `/api/internal/placeholder/{spec}`

Generate placeholder images on-the-fly for testing and development. Useful for validating stream configurations without external media sources.

**URL patterns:**
- `/api/internal/placeholder/64x64.png` - Gray square with dimensions as text
- `/api/internal/placeholder/600x400/orange.png` - Orange background, auto-contrast text
- `/api/internal/placeholder/600x400/ff0000/white.png` - Red background, white text
- `/api/internal/placeholder/800.png?text=Hello+World` - Custom text

**Parameters:**
- **Size**: `{width}x{height}` or `{size}` for square (10-4096px)
- **Background color** *(optional)*: CSS color name or hex (e.g., `orange`, `f00`, `ff0000`, `#00ff00`)
- **Text color** *(optional)*: CSS color name or hex (auto-contrasted if not specified)
- **Query param `text`** *(optional)*: Custom text (defaults to dimensions). Use `\n` for newlines.

**Internal Protocol Shorthand:**

For use with WebSocket `start_stream` commands, the `internal:` protocol provides a shorthand:

```json
{
  "type": "start_stream",
  "out": 1,
  "w": 64,
  "h": 64,
  "src": "internal:placeholder/64x64.png"
}
```

The `internal:` URL automatically resolves to `http://{server_host}/api/internal/placeholder/64x64.png` using the server's actual host and port.

**Examples:**
```bash
# Generate 64x64 gray placeholder
curl http://localhost:8788/api/internal/placeholder/64x64.png --output test.png

# Orange background with custom text
curl http://localhost:8788/api/internal/placeholder/600x400/orange.png?text=Test%20Image --output test.png
```

---

## Using it with ESPHome

See: [lvgl-ddp-stream](https://github.com/stuartparmenter/lvgl-ddp-stream) for ESPHome integration examples.

---

## Ports & networking

- **WebSocket control:** TCP `8788` (configurable).
- **DDP to devices:** UDP from the add-on to your displays on the port you specify per stream.

---

## Logs & troubleshooting

- Open the add-on’s **Log** tab to view server output.
- You’ll see decode path selection, filter-graph rebuilds, and metrics (fps, pps, jitter, drops).
- Common issues:
  - **No frames until page reload:** Ensure your client starts streaming after WebSocket connect.
  - **YouTube not playing:** The add-on must be able to reach YouTube; outbound internet must be allowed.
  - **Device not receiving frames:** Check UDP reachability and that pixel format matches your device.

---

## Rust compatibility checks

Run `cargo test --locked --all-targets` and, after `cargo build --locked --bins`,
`python3 tests/smoke.py --binary target/debug/media-proxy` (requires FFmpeg).
Tests compare all retained Python configuration defaults, animated disposal and
cached frame output, and real HTTP/WebSocket/DDP behavior. See
[the disposal fixture notes](tests/fixtures/animated/README.md) for reference details.

Home Assistant entity/template drawing was intentionally removed in the Rust
rewrite (`/api/internal/homeassistant/*` returns 501). The animimg frame-to-ZIP
utility has also been removed; it is outside the streaming service's scope.
