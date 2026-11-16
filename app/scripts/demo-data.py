#!/usr/bin/env python3
"""Fills a Mihomo-Monitor database with a year of made-up traffic, for README screenshots.

    python3 app/scripts/demo-data.py <traffic.db> <source key>

The app creates the database and the source (by connecting to scripts/fake-mihomo.py once);
this only adds history to that source, in the app's own 15-minute buckets. The numbers are
fictional: a daily rhythm, quieter weekends, the odd big download. The last 90 days are also
split by node, subscription, program and inbound, with the hosts of each day, using the names
fake-mihomo.py uses; older traffic stays unattributed, as for someone who upgraded from a
version without the split. Standard library only.
"""

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
    programs = [(name, w, hosts) for name, w, hosts in fake.PROGRAMS]
    direct_share = sum(w for name, w, _ in programs if name in fake.DIRECT_HOSTS)
    shares = {
        "outbound": [(n, (1 - direct_share) * w) for n, _, w in fake.NODES] + [("DIRECT", direct_share)],
        "subscription": [("白月光", (1 - direct_share) * 0.8), ("备用机场", (1 - direct_share) * 0.2), ("DIRECT", direct_share)],
        "process": [(name, w) for name, w, _ in programs],
        "inbound": [(name, w) for name, w in fake.INBOUNDS],
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
                rows.append((source, bucket, unattributed, up, down))
                continue
            seen = 1 - fake.UNSEEN
            rows.append((source, bucket, unattributed, int(up * fake.UNSEEN), int(down * fake.UNSEEN)))
            for kind, opts in shares.items():
                total = sum(w for _, w in opts)
                for key, w in opts:
                    f = w / total * seen * random.uniform(0.8, 1.2)
                    rows.append((source, bucket, ids[(kind, key)], int(up * f), int(down * f)))
        if back < SPLIT_DAYS:
            # Hosts cover what the listed connections carried: the day's traffic minus the unseen part.
            seen_up, seen_down = day_up * (1 - fake.UNSEEN), day_down * (1 - fake.UNSEEN)
            day = time.strftime("%Y-%m-%d", time.localtime(day_start))
            for name, w, hosts in programs:
                for i, host in enumerate(hosts):
                    f = w * (0.6 ** i) / sum(0.6 ** j for j in range(len(hosts))) * random.uniform(0.9, 1.1)
                    host_rows.append((source, day, ids[("process", name)], dim("domain", host), int(seen_up * f), int(seen_down * f)))
    db.executemany(
        "INSERT INTO traffic (source_id, bucket, dim_id, up, down) VALUES (?, ?, ?, ?, ?)"
        " ON CONFLICT (source_id, bucket, dim_id) DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
        rows,
    )
    db.executemany(
        "INSERT INTO domain_daily (source_id, day, process_dim_id, domain_dim_id, up, down) VALUES (?, ?, ?, ?, ?, ?)"
        " ON CONFLICT DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
        host_rows,
    )
    db.commit()
    print(f"added {len(rows)} traffic rows and {len(host_rows)} host rows to source {source}")


if __name__ == "__main__":
    main()
