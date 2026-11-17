#!/usr/bin/env python3
"""Fills a Mihomo-Monitor database with a year of made-up traffic, for README screenshots.

    python3 app/scripts/demo-data.py <traffic.db> <source key>

The app creates the database and the source (by connecting to scripts/fake-mihomo.py once);
this only adds history to that source, in the app's own 15-minute buckets. The numbers are
fictional: a daily rhythm, quieter weekends, the odd big download. The last 90 days are also
split by node, subscription, program and inbound, with the hosts of each day, using the names
fake-mihomo.py uses; older traffic stays unattributed, as for someone who upgraded from a
version without the split. The container fake-mihomo.py lists without a process is recorded
under the name of an attribution rule for its inbound, as if the rule had been there all along.
Standard library only.
"""

import json
import math
import os
import random
import sqlite3
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from importlib import import_module

fake = import_module("fake-mihomo")

BUCKET = 15 * 60
DAYS = 400
SPLIT_DAYS = 90
# Attribution rules: name, inbound, priority, enabled. The first names fake-mihomo.py's container.
RULES = [
    ("Container: us-workstation", "US-Workstation-Port", 10, True),
    ("Homelab NAS", "LAN-NAS-Port", 20, False),
]


def main():
    db_path, key = sys.argv[1], sys.argv[2]
    random.seed(7)
    db = sqlite3.connect(db_path)
    row = db.execute("SELECT id FROM sources WHERE key = ?", (key,)).fetchone()
    if not row:
        sys.exit(f"source {key!r} not found in {db_path}")
    source = row[0]
    def dim(kind, key):
        db.execute("INSERT OR IGNORE INTO dims (kind, key) VALUES (?, ?)", (kind, key))
        return db.execute("SELECT id FROM dims WHERE kind = ? AND key = ?", (kind, key)).fetchone()[0]

    unattributed = dim("outbound", "")
    # Each program as the app records it: a rule names what the core could not. Programs that go
    # out directly in fake-mihomo.py are direct, the rest go through a proxy.
    named = {inbound: name for name, inbound, _, on in RULES if on}

    def host_share(hosts, i):
        return 0.6**i / sum(0.6**j for j in range(len(hosts)))

    def host_route(program, host):
        return "direct" if program in fake.DIRECT_PROGRAMS or host in fake.DIRECT_SITES else "proxy"

    # (name as recorded, weight, hosts, dedicated inbound, share of it that goes direct)
    programs = []
    for i, (name, w, hosts) in enumerate(fake.PROGRAMS):
        inbound = fake.DEDICATED_INBOUND.get(i)
        direct = sum(host_share(hosts, j) for j, h in enumerate(hosts) if host_route(name, h) == "direct")
        programs.append((name or named.get(inbound, ""), w, hosts, inbound, direct))
    direct_share = sum(w * d for _, w, _, _, d in programs) / sum(w for _, w, _, _, _ in programs)

    # Each kind's shares as (key, route) -> weight.
    def add(table, key, route, w):
        table[(key, route)] = table.get((key, route), 0) + w

    inbounds = {}
    processes = {}
    for name, w, _, inbound, direct in programs:
        for route, part in (("proxy", 1 - direct), ("direct", direct)):
            if part <= 0:
                continue
            add(processes, name, route, w * part)
            for inb, iw in [(inbound, 1)] if inbound else fake.INBOUNDS:
                add(inbounds, inb, route, w * part * iw)
    shares = {
        "outbound": {**{(n, "proxy"): (1 - direct_share) * w for n, _, w in fake.NODES}, ("DIRECT", "direct"): direct_share},
        "subscription": {("白月光", "proxy"): (1 - direct_share) * 0.8, ("备用机场", "proxy"): (1 - direct_share) * 0.2, ("DIRECT", "direct"): direct_share},
        "process": processes,
        "inbound": inbounds,
    }
    ids = {(k, key): dim(k, key) for k, opts in shares.items() for key, _ in opts}

    now = int(time.time())
    local = time.localtime(now)
    midnight = now - (local.tm_hour * 3600 + local.tm_min * 60 + local.tm_sec)
    rows = []
    host_rows = []
    for back in range(DAYS, -1, -1):
        day_start = midnight - back * 86400
        weekday = time.localtime(day_start).tm_wday
        day_total = random.lognormvariate(math.log(2.6e9), 0.45) * (1.35 if weekday >= 5 else 1.0)
        if random.random() < 0.06:
            day_total += random.uniform(6e9, 14e9)
        day_up = day_down = 0
        for q in range(96):
            ts = day_start + q * BUCKET
            if ts + BUCKET > now:
                break
            hour = q / 4
            # Quiet at night, busy in the evening.
            shape = 0.08 + max(0.0, math.sin((hour - 7) / 17 * math.pi)) ** 1.4 + (0.6 if 20 <= hour < 23 else 0)
            down = int(day_total / 96 * shape * random.uniform(0.6, 1.4))
            up = int(down * random.uniform(0.05, 0.11))
            bucket = ts - ts % BUCKET
            day_up += up
            day_down += down
            if back >= SPLIT_DAYS:
                rows.append((source, bucket, unattributed, "", up, down))
                continue
            seen = 1 - fake.UNSEEN
            rows.append((source, bucket, unattributed, "", int(up * fake.UNSEEN), int(down * fake.UNSEEN)))
            for kind, opts in shares.items():
                total = sum(opts.values())
                # Every kind splits the same connections, so each adds up to what they carried.
                for (key, route), w in opts.items():
                    f = w / total * seen
                    rows.append((source, bucket, ids[(kind, key)], route, int(up * f), int(down * f)))
        if back < SPLIT_DAYS:
            # Hosts cover what the listed connections carried: the day's traffic minus the unseen part.
            seen_up, seen_down = day_up * (1 - fake.UNSEEN), day_down * (1 - fake.UNSEEN)
            day = time.strftime("%Y-%m-%d", time.localtime(day_start))
            for (name, w, hosts, _, _), (core_name, _, _) in zip(programs, fake.PROGRAMS):
                for i, host in enumerate(hosts):
                    f = w * host_share(hosts, i) * random.uniform(0.9, 1.1)
                    route = host_route(core_name, host)
                    host_rows.append((source, day, ids[("process", name)], dim("domain", host), route, int(seen_up * f), int(seen_down * f)))
    db.executemany(
        "INSERT INTO traffic (source_id, bucket, dim_id, route, up, down) VALUES (?, ?, ?, ?, ?, ?)"
        " ON CONFLICT (source_id, bucket, dim_id, route) DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
        rows,
    )
    db.executemany(
        "INSERT INTO domain_daily (source_id, day, process_dim_id, domain_dim_id, route, up, down) VALUES (?, ?, ?, ?, ?, ?, ?)"
        " ON CONFLICT DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
        host_rows,
    )
    # A monthly cycle for the main subscription, twelve days into it.
    reset_day = time.localtime(midnight - 12 * 86400).tm_mday
    db.execute(
        "INSERT OR REPLACE INTO cycles (subscription, anchor, quota_bytes, count_mode, alerts) VALUES (?, ?, ?, 'both', '[80,100]')",
        ("白月光", json.dumps({"kind": "monthly", "day": reset_day, "hour": 0, "minute": 0}), 200 * 1024**3),
    )
    # A few host groups; Apple, Steam and the rest stay ungrouped.
    groups = {
        "YouTube": [("suffix", "youtube.com"), ("suffix", "googlevideo.com"), ("suffix", "ytimg.com")],
        "Google": [("suffix", "google.com")],
        "Telegram": [("suffix", "telesco.pe"), ("cidr", "149.154.160.0/20")],
        "Spotify": [("suffix", "spotify.com"), ("domain", "audio-ak-spotify-com.akamaized.net")],
        "开发": [("suffix", "github.com"), ("suffix", "githubusercontent.com"), ("suffix", "visualstudio.com")],
    }
    for name, rules in groups.items():
        db.execute("INSERT OR IGNORE INTO host_groups (name, note, created_at) VALUES (?, '', ?)", (name, now))
        gid = db.execute("SELECT id FROM host_groups WHERE name = ?", (name,)).fetchone()[0]
        db.executemany("INSERT OR IGNORE INTO host_rules (group_id, kind, value) VALUES (?, ?, ?)", [(gid, k, v) for k, v in rules])
    db.executemany(
        "INSERT INTO attribution_rules (name, field, value, priority, enabled) VALUES (?, 'inbound', ?, ?, ?)",
        [(name, inbound, priority, int(on)) for name, inbound, priority, on in RULES],
    )
    db.commit()
    print(f"added {len(rows)} traffic rows and {len(host_rows)} host rows to source {source}")


if __name__ == "__main__":
    main()
