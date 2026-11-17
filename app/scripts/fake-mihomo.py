#!/usr/bin/env python3
"""A stand-in for mihomo's controller, for screenshots and trying the app without a real core.

    python3 app/scripts/fake-mihomo.py --unix /path/to/verge-mihomo.sock
    python3 app/scripts/fake-mihomo.py --port 19090 [--secret s3cret]

Answers `GET /version`, streams `GET /traffic` (one JSON line per second, with the cumulative
upTotal / downTotal), reports two subscriptions' usage on `GET /providers/proxies`, and lists
made-up open connections on `GET /connections`: a few programs
browsing, streaming and syncing through a handful of nodes from two subscriptions. Connections
open and close over time, and some traffic belongs to connections too short-lived to be listed,
as with a real core. Standard library only.
"""

import argparse
import itertools
import json
import math
import os
import random
import socketserver
import threading
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

VERSION = {"meta": True, "version": "v1.19.31"}

# Programs, the hosts they talk to, and how much of the traffic they make.
PROGRAMS = [
    ("Google Chrome", 0.38, ["www.youtube.com", "rr3---sn-a5mekn6d.googlevideo.com", "i.ytimg.com", "www.google.com", "github.com"]),
    ("Safari", 0.14, ["www.apple.com", "news.ycombinator.com", "developer.mozilla.org"]),
    ("Telegram", 0.10, ["149.154.167.51", "cdn4.telesco.pe"]),
    ("Code", 0.08, ["marketplace.visualstudio.com", "objects.githubusercontent.com"]),
    ("Spotify", 0.12, ["audio-ak-spotify-com.akamaized.net", "spclient.wg.spotify.com"]),
    ("WeChat", 0.06, ["szextshort.weixin.qq.com", "mmbiz.qpic.cn"]),
    ("Steam", 0.08, ["cdn.steamstatic.com", "store.steampowered.com"]),
    ("", 0.04, ["time.apple.com", "dns.google"]),
]
# Node, the subscription it came from ("" for none), and its share of proxied traffic.
NODES = [
    ("🇭🇰 香港 01", "白月光", 0.40),
    ("🇯🇵 日本 02", "白月光", 0.25),
    ("🇺🇸 美国 03", "白月光", 0.15),
    ("🇸🇬 新加坡 01", "备用机场", 0.20),
]
DIRECT_HOSTS = {"WeChat", "Steam", ""}
INBOUNDS = [("DEFAULT-MIXED", 0.7), ("DEFAULT-TUN", 0.3)]
# Share of the traffic carried by connections that open and close between two reads.
UNSEEN = 0.08
# What each subscription's provider reports: used GiB (upload, download) and allowance GiB.
REPORTS = {"白月光": (6, 86, 200), "备用机场": (2, 16, 100)}
GIB = 1024**3


def providers():
    now = time.time()
    return {
        "providers": {
            name: {
                "name": name,
                "type": "Proxy",
                "vehicleType": "HTTP",
                "proxies": [],
                "updatedAt": datetime.fromtimestamp(now - 3 * 3600, timezone.utc).isoformat(),
                "subscriptionInfo": {"Upload": up * GIB, "Download": down * GIB, "Total": total * GIB, "Expire": int(now + 40 * 86400)},
            }
            for name, (up, down, total) in REPORTS.items()
        }
    }


def pick(options, weight):
    """A random option, chosen in proportion to `weight(option)`."""
    r = random.random() * sum(weight(o) for o in options)
    for o in options:
        r -= weight(o)
        if r <= 0:
            return o
    return options[-1]


class Core:
    def __init__(self, up: int, down: int):
        self.lock = threading.Lock()
        self.up_total = up
        self.down_total = down
        self.rate = (0, 0)
        self.conns = {}
        self.ids = itertools.count(1)
        self.started = time.time()

    def _open(self):
        program, _, hosts = pick(PROGRAMS, lambda o: o[1])
        host = random.choice(hosts)
        if program in DIRECT_HOSTS:
            chains, providers = ["DIRECT"], [""]
        else:
            node, provider, _ = pick(NODES, lambda o: o[2])
            chains, providers = [node, "节点选择"], [provider, ""]
        cid = f"{next(self.ids):08x}-demo"
        ip = host if host[0].isdigit() else f"198.18.0.{random.randint(2, 250)}"
        self.conns[cid] = {
            "id": cid,
            "metadata": {
                "network": "tcp",
                "type": "HTTP",
                "sourceIP": "127.0.0.1",
                "destinationIP": ip,
                "host": "" if host[0].isdigit() else host,
                "process": program,
                "inboundName": pick(INBOUNDS, lambda o: o[1])[0],
            },
            "upload": 0,
            "download": 0,
            "start": datetime.now(timezone.utc).isoformat(),
            "chains": chains,
            "providerChains": providers,
            "rule": "Match",
            "weight": random.uniform(0.2, 3) * dict((p, w) for p, w, _ in PROGRAMS)[program],
        }

    def tick(self):
        t = time.time() - self.started
        browse = 180_000 + 140_000 * math.sin(t / 9) + random.uniform(0, 90_000)
        burst = 2_400_000 * max(0.0, math.sin(t / 23)) ** 6
        down = int(max(0, browse + burst))
        up = int(down * random.uniform(0.06, 0.12))
        with self.lock:
            while len(self.conns) < 12 or (len(self.conns) < 40 and random.random() < 0.4):
                self._open()
            for cid in [c for c in self.conns if random.random() < 0.04]:
                del self.conns[cid]
            seen_down, seen_up = int(down * (1 - UNSEEN)), int(up * (1 - UNSEEN))
            weights = sum(c["weight"] for c in self.conns.values())
            for c in self.conns.values():
                c["download"] += int(seen_down * c["weight"] / weights)
                c["upload"] += int(seen_up * c["weight"] / weights)
            self.up_total += up
            self.down_total += down
            self.rate = (up, down)

    def traffic(self):
        with self.lock:
            return {"up": self.rate[0], "down": self.rate[1], "upTotal": self.up_total, "downTotal": self.down_total}

    def connections(self):
        with self.lock:
            conns = [{k: v for k, v in c.items() if k != "weight"} for c in self.conns.values()]
            return {"downloadTotal": self.down_total, "uploadTotal": self.up_total, "connections": conns or None, "memory": 0}


def handler_for(core: Core, secret: str):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

        def address_string(self):
            return "local"

        def _authorized(self):
            if not secret or self.headers.get("Authorization") == f"Bearer {secret}":
                return True
            self.send_response(401)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return False

        def _json(self, data):
            body = json.dumps(data).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if not self._authorized():
                return
            if self.path == "/version":
                self._json(VERSION)
            elif self.path.startswith("/connections"):
                self._json(core.connections())
            elif self.path.startswith("/providers/proxies"):
                self._json(providers())
            elif self.path.startswith("/traffic"):
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
                try:
                    while True:
                        line = (json.dumps(core.traffic()) + "\n").encode()
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

    core = Core(a.up_total, a.down_total)

    def run():
        while True:
            core.tick()
            time.sleep(1)

    threading.Thread(target=run, daemon=True).start()
    if a.unix:
        if os.path.exists(a.unix):
            os.unlink(a.unix)
        server = UnixHTTPServer(a.unix, handler_for(core, ""))
        os.chmod(a.unix, 0o600)
    else:
        server = ThreadingHTTPServer(("127.0.0.1", a.port), handler_for(core, a.secret))
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
