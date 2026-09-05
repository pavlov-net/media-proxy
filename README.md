# Media Proxy

Media Proxy streams images and video to displays over DDP (UDP), controlled through
WebSocket messages. It runs as a standalone Rust binary or a
[Home Assistant add-on](https://github.com/stuartparmenter/media-proxy-addon).
For ESPHome displays, see [ddp-esphome](https://github.com/pavlov-net/ddp-esphome).

## Install and run

Linux releases contain `media-proxy` and `ddp-view` for x86-64 and aarch64. The
binaries use musl and statically linked Little CMS. The x86-64 build does not
require AVX2.

Download the archive and matching checksum from
[Releases](https://github.com/pavlov-net/media-proxy/releases). For example:

```sh
sha256sum -c media-proxy-1.0.0-x86_64-unknown-linux-musl.tar.gz.sha256
tar -xzf media-proxy-1.0.0-x86_64-unknown-linux-musl.tar.gz
./media-proxy --host 0.0.0.0 --port 8788
```

Video sources require `ffmpeg` and `ffprobe` on `PATH`. Allow outbound UDP to the
display's DDP port and inbound TCP to the server's control port. Run
`media-proxy --help` for CLI options.

### Web-page sources

Install [Deno](https://docs.deno.com/runtime/getting_started/installation/)
2.6.6 or newer and yt-dlp with its EJS solver:

```sh
uv tool install 'yt-dlp[default]'
yt-dlp --version
deno --version
```

Both executables must be on the service's `PATH`. Keep yt-dlp and its matching
`yt-dlp-ejs` dependency up to date for YouTube extraction. The `curl-cffi` extra
is optional for sites requiring browser impersonation. Direct media files and
streams do not require yt-dlp.

### Build from source

Install the toolchain specified in [rust-toolchain.toml](https://github.com/pavlov-net/media-proxy/blob/main/rust-toolchain.toml), then:

```sh
git clone https://github.com/pavlov-net/media-proxy.git
cd media-proxy
cargo build --locked --release
./target/release/media-proxy --host 0.0.0.0 --port 8788
```

## Configure and control streams

- [Configuration](docs/configuration.md): server settings, image/video processing,
  and resolver selection.
- [Control API](docs/api.md): WebSocket messages and HTTP endpoints.
- [Home Assistant setup](https://github.com/stuartparmenter/media-proxy-addon#installation):
  add-on installation and configuration paths.

Use `ddp-view --help` to configure a terminal DDP receiver for testing without a
physical display.

## Run checks

```sh
cargo fmt --package media-proxy -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --bins
python3 tests/smoke.py --binary target/debug/media-proxy
```

The smoke test requires FFmpeg. It exercises HTTP, WebSocket and DDP playback
using local media, including animation/video loop boundaries. With yt-dlp on
`PATH`, it also tests extraction from a local HTML page.
[Disposal fixtures](https://github.com/pavlov-net/media-proxy/blob/main/tests/fixtures/animated/README.md) use independent raw-subframe
compositing references.
