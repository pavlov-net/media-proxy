# Control API

WebSocket control uses `/control`. Each connection owns its streams; closing the
connection cancels them. DDP output uses UDP independently of the control socket.

## Connect

Connect to `ws://localhost:8788/control` and send `hello` as the first text message:

```json
{"type":"hello","device_id":"my_display"}
```

`device_id` is optional. The response has type `hello_ack` and a `server_version`
string containing `media-proxy/` followed by the crate version. The server sends
WebSocket ping frames; clients must handle the protocol's pong response.

## Start a stream

```json
{
  "type": "start_stream",
  "out": 5,
  "w": 64,
  "h": 64,
  "src": "https://example.com/video.mp4"
}
```

| Field | Contract |
| --- | --- |
| `out` | Required nonnegative output ID. DDP transmits its low eight bits. |
| `w`, `h` | Required nonzero pixel dimensions, bounded by `MAX_OUTPUT_DIM` in [field validation](https://github.com/pavlov-net/media-proxy/blob/main/src/control/fields.rs). |
| `src` | Required source: file path, `file:///absolute/path` URL, HTTP media/page URL, streaming URL, or `internal:` shorthand. |
| `ddp_host` | Destination IP address; omitted means the control client's address. Hostnames are not accepted. |
| `ddp_port` | Nonzero UDP destination port; omitted uses the DDP port in `StreamFields::from_start`. |
| `fit` | `auto`, `pad` or `cover`; omitted inherits `video.fit`. See [resize modes](configuration.md#resize-modes). |
| `loop` | Repeat playback; omitted inherits `playback.loop`. |
| `hw` | `auto`, `none`, `cuda`, `qsv`, `vaapi`, `videotoolbox` or `d3d11va`; omitted inherits `hw.prefer`. Available backends depend on the host and FFmpeg build. |
| `fmt` | `rgb888`, `rgb565le` or `rgb565be`; omitted uses RGB888. `rgbw` is accepted as RGB888 output. |
| `expand` | `0`: FFmpeg defaults; `1`: auto-detected input range to full range; `2`: limited input range to full range. Omitted inherits `video.expand_mode`. |
| `pace` | Integer sampling rate in Hz. Zero or omitted follows source cadence. |
| `ema` | Frame smoothing alpha in `[0, 1]` for paced playback; zero or omitted disables smoothing. |

File paths refer to the server's filesystem. Relative paths resolve from its
working directory; the Home Assistant add-on exposes media files under `/media`.

A successful request returns `ack` with `out` and an `applied` object containing
resolved stream parameters. The acknowledgement confirms acceptance; decoding
runs asynchronously. Stream failures appear in server logs.

Streams reserve a destination IP and eight-bit output ID. Starting another stream
with the same pair displaces the prior one, including across connections.

## Update or stop a stream

`update` restarts a stream with supplied fields; omitted fields retain their
values. The stream must belong to the connection.

```json
{"type":"update","out":5,"src":"https://example.com/animation.gif","loop":false}
```

`stop_stream` cancels the selected stream and returns `ack`, including when no
matching stream exists:

```json
{"type":"stop_stream","out":5}
```

Both messages accept `ddp_host` to select a destination other than the control
client's IP. Use the same destination as the start request.

## Ping and errors

JSON `ping` returns `pong` with the same `t` value. An omitted `t` is returned as
null. This message is separate from WebSocket ping/pong frames.

```json
{"type":"ping","t":123.5}
```

Errors have `type: error`, `code` and `message`. Invalid handshakes use `proto`;
invalid requests and unknown outputs use `bad_request`. See
[`ControlError`](https://github.com/pavlov-net/media-proxy/blob/main/src/error.rs) for the error mapping. Unknown JSON fields are
ignored.

## HTTP endpoints

### GET `/api/system/health`

Returns JSON with `status: ok` and `service: media-proxy`.

### GET `/api/internal/placeholder/{spec}`

Returns a PNG containing text. The specification accepts a square size or
`width`x`height`, optional background/text colors, and a `.png` suffix:

```text
/api/internal/placeholder/64x64.png
/api/internal/placeholder/600x400/orange.png
/api/internal/placeholder/600x400/ff0000/white.png
/api/internal/placeholder/800.png?text=Hello+World
```

Dimensions must be within the bounds enforced by
[`placeholder`](https://github.com/pavlov-net/media-proxy/blob/main/src/api/internal/placeholder.rs). Colors accept supported names
or hex; percent-encode `#` in a URL. Omitted colors use gray and contrasting text.
The `text` query value defaults to dimensions and accepts literal `\n` for line breaks.

For a stream source, `internal:placeholder/64x64.png` expands to the server's
`/api/internal/placeholder/64x64.png` URL using the control connection's Host header.

### GET `/api/internal/homeassistant/{entity}`

Returns 501. Home Assistant entity/template rendering is unsupported.
