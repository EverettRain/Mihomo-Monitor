#!/usr/bin/env python3
"""A stand-in for mihomo's controller, for screenshots and trying the app without a real core.

    python3 app/scripts/fake-mihomo.py --unix /path/to/verge-mihomo.sock
    python3 app/scripts/fake-mihomo.py --port 19090 [--secret s3cret]

Answers `GET /version` and streams `GET /traffic` (one JSON line per second, with the
cumulative upTotal / downTotal Mihomo-Monitor counts from). Rates follow a smooth made-up
pattern of browsing with an occasional download. Standard library only.
"""

import argparse
import json
import math
import os
import random
import socketserver
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

VERSION = {"meta": True, "version": "v1.19.31"}


class Counters:
    def __init__(self, up: int, down: int):
        self.up_total = up
        self.down_total = down
        self.started = time.time()

    def tick(self):
        t = time.time() - self.started
        browse = 180_000 + 140_000 * math.sin(t / 9) + random.uniform(0, 90_000)
        burst = 2_400_000 * max(0.0, math.sin(t / 23)) ** 6
        down = int(max(0, browse + burst))
        up = int(down * random.uniform(0.06, 0.12))
        self.up_total += up
        self.down_total += down
        return {"up": up, "down": down, "upTotal": self.up_total, "downTotal": self.down_total}


def handler_for(counters: Counters, secret: str):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

        def address_string(self):
            return "local"

        def _authorized(self):
            if not secret:
                return True
            if self.headers.get("Authorization") == f"Bearer {secret}":
                return True
            self.send_response(401)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return False

        def do_GET(self):
            if not self._authorized():
                return
            if self.path == "/version":
                body = json.dumps(VERSION).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif self.path.startswith("/traffic"):
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
                try:
                    while True:
                        line = (json.dumps(counters.tick()) + "\n").encode()
                        self.wfile.write(f"{len(line):x}\r\n".encode() + line + b"\r\n")
                        self.wfile.flush()
                        time.sleep(1)
                except (BrokenPipeError, ConnectionResetError):
                    pass
            else:
                self.send_response(404)
                self.send_header("Content-Length", "0")
                self.end_headers()

    return Handler


class UnixHTTPServer(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True

    def get_request(self):
        request, _ = super().get_request()
        return request, ("local", 0)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    where = p.add_mutually_exclusive_group(required=True)
    where.add_argument("--unix", help="listen on this unix socket")
    where.add_argument("--port", type=int, help="listen on 127.0.0.1:PORT")
    p.add_argument("--secret", default="", help="require this secret (HTTP only)")
    # A real core counts from zero when it starts.
    p.add_argument("--up-total", type=int, default=0)
    p.add_argument("--down-total", type=int, default=0)
    a = p.parse_args()

    counters = Counters(a.up_total, a.down_total)
    if a.unix:
        if os.path.exists(a.unix):
            os.unlink(a.unix)
        server = UnixHTTPServer(a.unix, handler_for(counters, ""))
        os.chmod(a.unix, 0o600)
    else:
        server = ThreadingHTTPServer(("127.0.0.1", a.port), handler_for(counters, a.secret))
        server.daemon_threads = True
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        if a.unix and os.path.exists(a.unix):
            os.unlink(a.unix)


if __name__ == "__main__":
    main()
