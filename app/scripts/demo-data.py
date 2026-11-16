#!/usr/bin/env python3
"""Fills a Mihomo-Monitor database with a year of made-up traffic, for README screenshots.

    python3 app/scripts/demo-data.py <traffic.db> <source key>

The app creates the database and the source (by connecting to scripts/fake-mihomo.py once);
this only adds history to that source, in the app's own 15-minute buckets. The numbers are
fictional: a daily rhythm, quieter weekends, the odd big download. Standard library only.
"""

import math
import random
import sqlite3
import sys
import time

BUCKET = 15 * 60
DAYS = 400


def main():
    db_path, key = sys.argv[1], sys.argv[2]
    random.seed(7)
    db = sqlite3.connect(db_path)
    row = db.execute("SELECT id FROM sources WHERE key = ?", (key,)).fetchone()
    if not row:
        sys.exit(f"source {key!r} not found in {db_path}")
    source = row[0]
    dim = db.execute("SELECT id FROM dims WHERE kind = 'outbound' AND key = ''").fetchone()[0]

    now = int(time.time())
    local = time.localtime(now)
    midnight = now - (local.tm_hour * 3600 + local.tm_min * 60 + local.tm_sec)
    rows = []
    for back in range(DAYS, -1, -1):
        day_start = midnight - back * 86400
        weekday = time.localtime(day_start).tm_wday
        day_total = random.lognormvariate(math.log(2.6e9), 0.45) * (1.35 if weekday >= 5 else 1.0)
        if random.random() < 0.06:
            day_total += random.uniform(6e9, 14e9)
        for q in range(96):
            ts = day_start + q * BUCKET
            if ts + BUCKET > now:
                break
            hour = q / 4
            # Quiet at night, busy in the evening.
            shape = 0.08 + max(0.0, math.sin((hour - 7) / 17 * math.pi)) ** 1.4 + (0.6 if 20 <= hour < 23 else 0)
            down = int(day_total / 96 * shape * random.uniform(0.6, 1.4))
            up = int(down * random.uniform(0.05, 0.11))
            rows.append((source, ts - ts % BUCKET, dim, up, down))
    db.executemany(
        "INSERT INTO traffic (source_id, bucket, dim_id, up, down) VALUES (?, ?, ?, ?, ?)"
        " ON CONFLICT (source_id, bucket, dim_id) DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
        rows,
    )
    db.commit()
    print(f"added {len(rows)} buckets to source {source}")


if __name__ == "__main__":
    main()
