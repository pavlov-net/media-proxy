# Configuration

`--config` loads YAML, TOML or JSON. Missing values use the defaults in
[`Config`](https://github.com/pavlov-net/media-proxy/blob/main/src/config.rs). Environment variables override file values using
`MEDIA_PROXY__SECTION__KEY`, such as `MEDIA_PROXY__IMAGE__GAMMA_CORRECT=true`.
The listening address and port are CLI options; see `media-proxy --help`.

This example disables hardware decoding and enables video border detection:

```yaml
hw:
  prefer: none
video:
  autocrop:
    enabled: true
```

## Processing settings

| Setting | Meaning |
| --- | --- |
| `hw.prefer` | Hardware preference when a stream omits `hw`; see [stream fields](api.md#start-a-stream). |
| `video.fit` | Resize mode when a stream omits `fit`. |
| `video.expand_mode` | Color-range expansion when a stream omits `expand`. |
| `video.autocrop.enabled` | Detect video borders during startup. Dark opening scenes can be mistaken for borders. |
| `video.autocrop.probe_frames` | Number of grayscale frames to sample. |
| `video.autocrop.luma_thresh` | Luma threshold for classifying a row or column as a border. |
| `video.autocrop.max_bar_ratio` | Maximum fraction of each edge considered for cropping. |
| `video.autocrop.min_bar_px` | Minimum detected border width in source pixels. |
| `playback.loop` | Loop setting when a stream omits `loop`. |
| `image.method` | Resampling filter: `lanczos`, `bicubic`, `bilinear`, `box`, `nearest` or `auto`. |
| `image.gamma_correct` | Resize in linear-light RGB. |
| `image.color_correction` | Convert supported embedded ICC profiles to sRGB. |
| `image.unsharp.amount`, `radius`, `threshold` | Sharpening controls; zero amount disables sharpening. |
| `image.frame_cache_mb` | Processed animation cache budget in MiB; zero disables retention. |
| `image.frame_cache_min_frames` | Minimum sequence length eligible for caching. |
| `playback_still.redundancy` | Copies per packet for the first frame of non-looping native playback, clamped to at least one. Looping and paced playback send one copy. |

### Resize modes

`pad` preserves the source aspect ratio and fills uncovered pixels with black.
`cover` fills the display and crops excess content. `auto` scales directly when
source and display aspect ratios are close; otherwise it uses `pad`.

Autocrop samples the beginning of a video at reduced resolution, takes the median
border width for each edge, and applies that crop throughout playback. A failed
probe leaves the video uncropped.

### YouTube formats and caching

`youtube.60fps` tries 720p 60fps formats before resolution-matched alternatives.
Disable it to prioritize resolution matching. Format selection considers target
height and hardware preference; the exact codec/resolution order is defined by
[`build_format`](https://github.com/pavlov-net/media-proxy/blob/main/src/yt_dlp/format.rs).

`youtube.cache.enabled` lets looping videos with a known size at or below
`youtube.cache.max_size` use FFmpeg's file cache. The size limit is in bytes.

## Resolver selection

An explicit `resolver.url` takes precedence over local yt-dlp. Without it, the
server detects yt-dlp on `PATH`. Direct media bypasses extraction. Startup logs
identify the selected resolver.

```yaml
resolver:
  url: http://127.0.0.1:8790/resolve
  timeout_ms: 30000
```

The external endpoint accepts JSON POST requests and returns a stream URL with
optional headers and metadata. See [`ResolveRequest` and `ResolveResponse`](https://github.com/pavlov-net/media-proxy/blob/main/src/resolver/mod.rs)
for the schema. `resolver.timeout_ms` bounds extraction in milliseconds.

## Network and logging settings

| Setting | Meaning |
| --- | --- |
| `net.user_agent` | HTTP User-Agent for media requests unless extraction supplies one. |
| `net.spread_packets` | Spread a frame's DDP packets across its frame interval. |
| `net.spread_max_fps` | Disable spreading above this frame rate. |
| `net.spread_min_ms` | Minimum spacing between packet groups in milliseconds. |
| `net.spread_max_sleeps` | Limit sleeps per frame; zero leaves the count uncapped. |
| `net.win_timer_res` | Request a finer Windows timer resolution. |
| `log.level` | Logging severity; `--log-level` overrides it. |
| `log.metrics` | Enable periodic DDP sender metrics. |
| `log.rate_ms` | Metrics reporting interval in milliseconds. |
| `log.send_ms` | Accepted configuration key with no runtime effect. |
