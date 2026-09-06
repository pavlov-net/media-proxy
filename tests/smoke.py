#!/usr/bin/env python3
"""Exercise the shipping binary over HTTP, WebSocket, and UDP. Requires ffmpeg.

No Python packages or external services are needed. Run with:
    python3 tests/smoke.py --binary target/debug/media-proxy
"""
import argparse
import base64
import contextlib
import hashlib
import http.server
import json
import os
from pathlib import Path
import signal
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import zlib


def png(rgb, width=16, height=16):
    def chunk(kind, body):
        return struct.pack('!I', len(body)) + kind + body + struct.pack('!I', zlib.crc32(kind + body))
    data = (b'\0' + bytes(rgb) * width) * height
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!2I5B', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(data)) + chunk(b'IEND', b''))


class WebSocket:
    def __init__(self, port):
        self.sock = socket.create_connection(('127.0.0.1', port), timeout=10)
        self.file = self.sock.makefile('rb')
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f'GET /control HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n'
                           f'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n'
                           'Sec-WebSocket-Version: 13\r\n\r\n').encode())
        assert b'101' in self.file.readline()
        headers = {}
        while (line := self.file.readline()) != b'\r\n':
            name, value = line.decode().strip().split(':', 1)
            headers[name.lower()] = value.strip()
        expected = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
        assert headers['sec-websocket-accept'] == expected
        assert self.request({'type': 'hello', 'device_id': 'release-smoke', 'proto': 'ddp-ws/1'})['type'] == 'hello_ack'

    def send(self, data, opcode=1):
        mask = os.urandom(4)
        size = len(data)
        prefix = bytes([0x80 | opcode, 0x80 | size]) if size < 126 else bytes([0x80 | opcode, 0xfe]) + struct.pack('!H', size)
        self.sock.sendall(prefix + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def request(self, message):
        self.send(json.dumps(message).encode())
        while True:
            head = self.file.read(2)
            assert len(head) == 2, 'WebSocket closed unexpectedly'
            opcode, length = head[0] & 15, head[1] & 127
            if length == 126:
                length = struct.unpack('!H', self.file.read(2))[0]
            elif length == 127:
                length = struct.unpack('!Q', self.file.read(8))[0]
            payload = self.file.read(length)
            if opcode == 9:
                self.send(payload, 10)
            elif opcode == 1:
                return json.loads(payload)
            else:
                assert opcode == 10, f'unexpected opcode {opcode}'

    def close(self):
        with contextlib.suppress(OSError):
            self.send(b'', 8)
        self.file.close()
        self.sock.close()


def receive_before(udp, deadline):
    remaining = deadline - time.monotonic()
    assert remaining > 0, 'DDP playback exceeded its deadline'
    udp.settimeout(min(10, remaining))
    return udp.recv(65535)


def run(binary):
    with tempfile.TemporaryDirectory(prefix='media-proxy-smoke-') as temp:
        root = Path(temp)
        (root / 'red.png').write_bytes(png((255, 0, 0)))
        (root / 'blue.png').write_bytes(png((0, 0, 255)))
        (root / 'wide.png').write_bytes(png((255, 0, 0), width=16, height=8))
        subprocess.run(['ffmpeg', '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=32x32:rate=30',
                        '-t', '2', '-c:v', 'mpeg4', str(root / 'clip.mp4')], check=True)
        for name, extra in [('anim.gif', []), ('anim.png', ['-f', 'apng']), ('anim.webp', ['-c:v', 'libwebp_anim'])]:
            subprocess.run(['ffmpeg', '-v', 'error', '-i', str(root / 'clip.mp4'), '-t', '1',
                            *extra, str(root / name)], check=True)
        class FixtureHandler(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                super().__init__(*args, directory=str(root), **kwargs)

            def translate_path(self, path):
                if path == '/stream':
                    return str(root / 'clip.mp4')
                return super().translate_path(path)

            def do_HEAD(self):
                if self.path == '/stream' and self.headers.get('User-Agent') != 'media-proxy-smoke/1':
                    self.send_error(403)
                    return
                super().do_HEAD()

            def do_GET(self):
                if self.path == '/stream' and self.headers.get('User-Agent') != 'media-proxy-smoke/1':
                    self.send_error(403)
                    return
                super().do_GET()

            def log_message(self, *args):
                pass

        fixture = http.server.ThreadingHTTPServer(('127.0.0.1', 0), FixtureHandler)
        threading.Thread(target=fixture.serve_forever, daemon=True).start()
        media_url = f'http://127.0.0.1:{fixture.server_port}'
        (root / 'watch.html').write_text('<html><title>Local video</title><video src="clip.mp4"></video></html>')
        (root / 'config.yaml').write_text('hw:\n  prefer: none\nlog:\n  level: INFO\nnet:\n  user_agent: media-proxy-smoke/1\n')
        with socket.socket() as reserve:
            reserve.bind(('127.0.0.1', 0))
            port = reserve.getsockname()[1]
        url = f'http://127.0.0.1:{port}'
        with (root / 'server.log').open('w+') as log:
            process = subprocess.Popen([str(binary), '--host', '127.0.0.1', '--port', str(port),
                                        '--config', str(root / 'config.yaml'), '--log-level', 'INFO'], stdout=log, stderr=log)
            try:
                for _ in range(100):
                    try:
                        with urllib.request.urlopen(url + '/api/system/health', timeout=0.5) as response:
                            assert json.load(response) == {'status': 'ok', 'service': 'media-proxy'}
                        break
                    except (OSError, urllib.error.URLError):
                        assert process.poll() is None, 'server exited during startup'
                        time.sleep(0.05)
                else:
                    raise AssertionError('server did not become healthy')

                with urllib.request.urlopen(url + '/api/internal/placeholder/64x64.png?text=Hello') as response:
                    assert response.read().startswith(b'\x89PNG')

                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
                    udp.bind(('127.0.0.1', 0))
                    udp.settimeout(10)
                    ws = WebSocket(port)
                    try:
                        assert ws.request({'type': 'ping', 't': 123.5}) == {'type': 'pong', 't': 123.5}
                        for fmt, pixel in [('rgb888', b'\xff\0\0'), ('rgb565le', b'\0\xf8'), ('rgb565be', b'\xf8\0'), ('rgbw', b'\xff\0\0')]:
                            response = ws.request(dict(type='start_stream', out=7, w=64, h=64, src=str(root / 'red.png'),
                                                       loop=False, fmt=fmt, pixcfg={'rgb888': 0x0b, 'rgb565le': 0x62, 'rgb565be': 0x61, 'rgbw': 0x1b}[fmt], ddp_port=udp.getsockname()[1]))
                            assert response['type'] == 'ack', response
                            assert response['applied']['hw'] == 'none', response
                            expected = pixel * (64 * 64)
                            assembled = bytearray(len(expected))
                            packet_count = (len(expected) + 1439) // 1440
                            pushes = 0
                            for _ in range(packet_count * 3):
                                packet = udp.recv(65535)
                                assert packet[3] == 7 and len(packet) <= 1450
                                assert packet[0] & 0x40 and 1 <= packet[1] <= 15
                                assert packet[2] == {'rgb888': 0x0b, 'rgbw': 0x0b, 'rgb565le': 0x62, 'rgb565be': 0x61}[fmt]
                                offset, length = struct.unpack('!IH', packet[4:10])
                                assert offset % len(pixel) == length % len(pixel) == 0
                                assert length == len(packet) - 10
                                assembled[offset:offset + length] = packet[10:]
                                if packet[0] & 1:
                                    assert offset + length == len(expected)
                                    pushes += 1
                            assert assembled == expected, fmt
                            assert pushes == 3  # default still redundancy
                        response = ws.request(dict(type='update', out=7, w=16, h=16, src=str(root / 'blue.png'), fmt='rgb888'))
                        assert response['type'] == 'ack', response
                        for _ in range(3):
                            assert udp.recv(65535)[10:] == b'\0\0\xff' * 256
                        # Omit playback overrides to exercise the defaults used by ddp-esphome.
                        sources = [str(root / name) for name in ['red.png', 'wide.png', 'anim.gif', 'anim.png', 'anim.webp', 'clip.mp4']]
                        sources += [media_url + '/stream']
                        if shutil.which('yt-dlp'):
                            sources.append(media_url + '/watch.html')
                        for source in sources:
                            response = ws.request(dict(type='start_stream', out=9, w=16, h=16,
                                                       src=source, ddp_port=udp.getsockname()[1]))
                            assert response['type'] == 'ack', response
                            applied = response['applied']
                            assert applied['loop'] is True and applied['fit'] == 'auto'
                            assert applied['fmt'] == 'rgb888' and applied['expand'] == 2
                            assert applied['pace'] == 0 and applied['ema'] == 0
                            is_animation = source.endswith(('anim.gif', 'anim.png', 'anim.webp'))
                            is_still = source.endswith(('red.png', 'wide.png'))
                            # Fixtures contain 30 animation / 60 video frames.
                            # Read past EOF to verify the default looping path.
                            frames = 5 if is_still else 35 if is_animation else 65
                            deadline = time.monotonic() + 15
                            first_received = None
                            for _ in range(frames):
                                packet = receive_before(udp, deadline)
                                if first_received is None:
                                    first_received = time.monotonic()
                                assert packet[3] == 9 and len(packet) == 778, (source, packet[:10])
                                if source.endswith('wide.png'):
                                    assert packet[10:] == bytes(16 * 4 * 3) + b'\xff\0\0' * (16 * 8) + bytes(16 * 4 * 3)
                            if not is_still:
                                assert time.monotonic() - first_received >= (0.75 if is_animation else 1.5), 'frames emitted too quickly'
                            assert ws.request(dict(type='stop_stream', out=9))['type'] == 'ack'
                            udp.settimeout(0.2)
                            while True:
                                try:
                                    udp.recv(65535)
                                except socket.timeout:
                                    break
                            udp.settimeout(10)
                        response = ws.request(dict(type='start_stream', out=10, w=16, h=16,
                                                   src=str(root / 'anim.gif'), pace=30, ddp_port=udp.getsockname()[1]))
                        assert response['type'] == 'ack', response
                        deadline = time.monotonic() + 10
                        for _ in range(35):
                            assert receive_before(udp, deadline)[3] == 10
                        assert ws.request(dict(type='stop_stream', out=10))['type'] == 'ack'
                        udp.settimeout(0.2)
                        while True:
                            try:
                                udp.recv(65535)
                            except socket.timeout:
                                break
                        udp.settimeout(10)
                        response = ws.request(dict(type='start_stream', out=11, w=16, h=16,
                                                   src=str(root / 'blue.png'), pace=1, loop=False,
                                                   ddp_port=udp.getsockname()[1]))
                        assert response['type'] == 'ack', response
                        packet = udp.recv(65535)
                        assert packet[3] == 11 and packet[10:] == b'\0\0\xff' * 256
                        assert ws.request(dict(type='stop_stream', out=11))['type'] == 'ack'
                        response = ws.request(dict(type='start_stream', out=8, w=16, h=16, src=str(root / 'clip.mp4'),
                                                   loop=True, ddp_port=udp.getsockname()[1]))
                        assert response['type'] == 'ack', response
                        assert udp.recv(65535)[3] == 8
                        assert ws.request(dict(type='stop_stream', out=8))['type'] == 'ack'
                        udp.settimeout(0.3)
                        for _ in range(100):
                            try:
                                udp.recv(65535)
                            except socket.timeout:
                                break
                        else:
                            raise AssertionError('DDP continued after stop_stream')
                    finally:
                        ws.close()
                process.send_signal(signal.SIGTERM)
                assert process.wait(timeout=10) == 0
                if shutil.which('yt-dlp'):
                    print('PASS: real yt-dlp local HTML extraction to DDP')
                print('PASS: HTTP health/PNG, default still/GIF/APNG/WebP/video streaming, WebSocket updates, RGB888/RGB565 DDP, stop, SIGTERM')
            except BaseException:
                log.flush()
                log.seek(0)
                print(log.read())
                raise
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                fixture.shutdown()
                fixture.server_close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/media-proxy'))
    run(parser.parse_args().binary.resolve())
