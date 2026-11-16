//! The statistics database (SQLite).
//!
//! Traffic lives in 15-minute buckets per source and dimension, kept forever. A dimension is one
//! way of splitting traffic (outbound node, subscription, process, inbound); the outbound rows of
//! a bucket always add up to its total, with traffic that could not be attributed under the
//! empty outbound key. Schema changes go through `migrate`, keyed by `PRAGMA user_version`.

use crate::domain::counter::{Bytes, Checkpoint, CoreId};
use crate::domain::period::Range;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const SCHEMA_VERSION: i32 = 1;

const SCHEMA_V1: &str = "
CREATE TABLE sources (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE dims (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    key TEXT NOT NULL,
    UNIQUE (kind, key)
);
CREATE TABLE traffic (
    source_id INTEGER NOT NULL,
    bucket INTEGER NOT NULL,
    dim_id INTEGER NOT NULL,
    up INTEGER NOT NULL,
    down INTEGER NOT NULL,
    PRIMARY KEY (source_id, bucket, dim_id)
) WITHOUT ROWID;
CREATE TABLE domain_daily (
    source_id INTEGER NOT NULL,
    day TEXT NOT NULL,
    process_dim_id INTEGER NOT NULL,
    domain_dim_id INTEGER NOT NULL,
    up INTEGER NOT NULL,
    down INTEGER NOT NULL,
    PRIMARY KEY (source_id, day, process_dim_id, domain_dim_id)
) WITHOUT ROWID;
CREATE TABLE checkpoints (
    source_id INTEGER PRIMARY KEY,
    core_pid INTEGER,
    core_started_at INTEGER,
    up_total INTEGER NOT NULL,
    down_total INTEGER NOT NULL,
    at INTEGER NOT NULL
);
CREATE TABLE cycles (
    id INTEGER PRIMARY KEY,
    subscription TEXT NOT NULL,
    anchor TEXT NOT NULL,
    quota_bytes INTEGER,
    count_mode TEXT NOT NULL,
    alerts TEXT NOT NULL
);
";

/// Traffic to add to one bucket of a source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BucketDelta {
    pub source: i64,
    pub bucket: i64,
    pub bytes: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRow {
    pub id: i64,
    pub key: String,
    pub name: String,
}

pub struct Store {
    conn: Connection,
    /// Dimension row of unattributed outbound traffic: all traffic until per-connection
    /// collection splits it.
    unattributed: i64,
}

/// Byte counts are stored as SQLite integers (i64); real traffic never comes near the limit.
fn bytes_at(r: &rusqlite::Row, up: usize) -> rusqlite::Result<Bytes> {
    Ok(Bytes::new(r.get::<_, i64>(up)?.max(0) as u64, r.get::<_, i64>(up + 1)?.max(0) as u64))
}

fn int(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn err(e: rusqlite::Error) -> String {
    format!("数据库错误：{e}")
}

impl Store {
    pub fn open(path: &Path) -> Result<Store, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("无法创建数据目录：{e}"))?;
        }
        let conn = Connection::open(path).map_err(err)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(err)?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(err)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Store {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> Result<Store, String> {
        migrate(&conn)?;
        conn.execute("INSERT OR IGNORE INTO dims (kind, key) VALUES ('outbound', '')", []).map_err(err)?;
        let unattributed = conn.query_row("SELECT id FROM dims WHERE kind = 'outbound' AND key = ''", [], |r| r.get(0)).map_err(err)?;
        Ok(Store { conn, unattributed })
    }

    /// The id of the source with this key, creating it (or updating its display name).
    pub fn ensure_source(&self, key: &str, name: &str, now: i64) -> Result<i64, String> {
        self.conn
            .query_row(
                "INSERT INTO sources (key, name, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (key) DO UPDATE SET name = excluded.name
                 RETURNING id",
                params![key, name, now],
                |r| r.get(0),
            )
            .map_err(err)
    }

    pub fn sources(&self) -> Result<Vec<SourceRow>, String> {
        let mut stmt = self.conn.prepare("SELECT id, key, name FROM sources ORDER BY id").map_err(err)?;
        let rows = stmt.query_map([], |r| Ok(SourceRow { id: r.get(0)?, key: r.get(1)?, name: r.get(2)? })).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn checkpoint(&self, source: i64) -> Result<Option<Checkpoint>, String> {
        self.conn
            .query_row("SELECT core_pid, core_started_at, up_total, down_total, at FROM checkpoints WHERE source_id = ?1", [source], |r| {
                let pid: Option<u32> = r.get(0)?;
                let started: Option<i64> = r.get(1)?;
                Ok(Checkpoint {
                    core: pid.zip(started).map(|(pid, started_at)| CoreId { pid, started_at }),
                    totals: bytes_at(r, 2)?,
                    at: r.get(4)?,
                })
            })
            .optional()
            .map_err(err)
    }

    /// Adds the traffic and moves the checkpoints in one transaction: after a crash either both
    /// are on disk or neither is, so the next start neither loses nor double-counts traffic.
    pub fn commit(&mut self, deltas: &[BucketDelta], checkpoints: &[(i64, Checkpoint)]) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(err)?;
        {
            let mut add = tx
                .prepare_cached(
                    "INSERT INTO traffic (source_id, bucket, dim_id, up, down) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (source_id, bucket, dim_id) DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
                )
                .map_err(err)?;
            for d in deltas.iter().filter(|d| !d.bytes.is_zero()) {
                add.execute(params![d.source, d.bucket, self.unattributed, int(d.bytes.up), int(d.bytes.down)]).map_err(err)?;
            }
            let mut mark = tx
                .prepare_cached(
                    "INSERT OR REPLACE INTO checkpoints (source_id, core_pid, core_started_at, up_total, down_total, at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(err)?;
            for (source, c) in checkpoints {
                let core = c.core.as_ref();
                mark.execute(params![source, core.map(|c| c.pid), core.map(|c| c.started_at), int(c.totals.up), int(c.totals.down), c.at])
                    .map_err(err)?;
            }
        }
        tx.commit().map_err(err)
    }

    /// Total traffic in `range`, of one source or of all.
    pub fn sum(&self, range: Range, source: Option<i64>) -> Result<Bytes, String> {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(t.up), 0), COALESCE(SUM(t.down), 0)
                 FROM traffic t JOIN dims d ON d.id = t.dim_id
                 WHERE d.kind = 'outbound' AND t.bucket >= ?1 AND t.bucket < ?2
                   AND (?3 IS NULL OR t.source_id = ?3)",
                params![range.start, range.end, source],
                |r| bytes_at(r, 0),
            )
            .map_err(err)
    }

    /// First and last bucket holding traffic, of one source or of all.
    pub fn bounds(&self, source: Option<i64>) -> Result<Option<(i64, i64)>, String> {
        self.conn
            .query_row("SELECT MIN(bucket), MAX(bucket) FROM traffic WHERE ?1 IS NULL OR source_id = ?1", [source], |r| {
                Ok(r.get::<_, Option<i64>>(0)?.zip(r.get::<_, Option<i64>>(1)?))
            })
            .map_err(err)
    }

    /// Total traffic per source in `range`; sources without traffic are left out.
    pub fn sums_by_source(&self, range: Range) -> Result<Vec<(i64, Bytes)>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT t.source_id, SUM(t.up), SUM(t.down)
                 FROM traffic t JOIN dims d ON d.id = t.dim_id
                 WHERE d.kind = 'outbound' AND t.bucket >= ?1 AND t.bucket < ?2
                 GROUP BY t.source_id",
            )
            .map_err(err)?;
        let rows = stmt.query_map(params![range.start, range.end], |r| Ok((r.get(0)?, bytes_at(r, 1)?))).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Per-bucket totals in `range`, oldest first; empty buckets are left out.
    pub fn buckets(&self, range: Range, source: Option<i64>) -> Result<Vec<(i64, Bytes)>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT t.bucket, SUM(t.up), SUM(t.down)
                 FROM traffic t JOIN dims d ON d.id = t.dim_id
                 WHERE d.kind = 'outbound' AND t.bucket >= ?1 AND t.bucket < ?2
                   AND (?3 IS NULL OR t.source_id = ?3)
                 GROUP BY t.bucket ORDER BY t.bucket",
            )
            .map_err(err)?;
        let rows = stmt.query_map(params![range.start, range.end, source], |r| Ok((r.get(0)?, bytes_at(r, 1)?))).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(err)?;
    if version > SCHEMA_VERSION {
        return Err("数据库来自更新版本的 Mihomo-Monitor，请升级后再打开。".into());
    }
    if version < 1 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;")).map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: i64 = crate::domain::period::BUCKET_SECS;

    fn delta(source: i64, bucket: i64, up: u64, down: u64) -> BucketDelta {
        BucketDelta { source, bucket, bytes: Bytes::new(up, down) }
    }

    #[test]
    fn ensure_source_is_stable_per_key_and_follows_renames() {
        // Arrange
        let store = Store::open_in_memory();

        // Act
        let first = store.ensure_source("clash-verge-rev", "Clash Verge Rev", 1).unwrap();
        let again = store.ensure_source("clash-verge-rev", "Verge", 2).unwrap();
        let other = store.ensure_source("manual:http://10.0.0.1:9090", "路由器", 3).unwrap();

        // Assert
        assert_eq!(first, again);
        assert_ne!(first, other);
        assert_eq!(store.sources().unwrap()[0].name, "Verge");
    }

    #[test]
    fn commits_add_up_within_a_bucket() {
        // Arrange
        let mut store = Store::open_in_memory();
        let s = store.ensure_source("a", "A", 0).unwrap();

        // Act
        store.commit(&[delta(s, 0, 10, 100)], &[]).unwrap();
        store.commit(&[delta(s, 0, 5, 50), delta(s, B, 1, 1)], &[]).unwrap();

        // Assert
        assert_eq!(store.buckets(Range { start: 0, end: 2 * B }, Some(s)).unwrap(), vec![(0, Bytes::new(15, 150)), (B, Bytes::new(1, 1))]);
    }

    #[test]
    fn sum_respects_the_range_and_the_source() {
        // Arrange
        let mut store = Store::open_in_memory();
        let a = store.ensure_source("a", "A", 0).unwrap();
        let b = store.ensure_source("b", "B", 0).unwrap();
        store.commit(&[delta(a, 0, 1, 2), delta(a, B, 10, 20), delta(b, B, 100, 200), delta(a, 2 * B, 1000, 2000)], &[]).unwrap();
        let range = Range { start: B, end: 2 * B };

        // Act
        let of_a = store.sum(range, Some(a)).unwrap();
        let of_all = store.sum(range, None).unwrap();

        // Assert
        assert_eq!(of_a, Bytes::new(10, 20));
        assert_eq!(of_all, Bytes::new(110, 220));
    }

    #[test]
    fn checkpoints_round_trip_with_and_without_a_core() {
        // Arrange
        let mut store = Store::open_in_memory();
        let local = store.ensure_source("a", "A", 0).unwrap();
        let remote = store.ensure_source("b", "B", 0).unwrap();
        let with_core = Checkpoint { core: Some(CoreId { pid: 42, started_at: 1_000 }), totals: Bytes::new(7, 8), at: 2_000 };
        let without = Checkpoint { core: None, totals: Bytes::new(1, 2), at: 3_000 };

        // Act
        store.commit(&[], &[(local, with_core.clone()), (remote, without.clone())]).unwrap();

        // Assert
        assert_eq!(store.checkpoint(local).unwrap(), Some(with_core));
        assert_eq!(store.checkpoint(remote).unwrap(), Some(without));
        assert_eq!(store.checkpoint(999).unwrap(), None);
    }

    #[test]
    fn reopening_keeps_data_and_schema() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traffic.db");
        {
            let mut store = Store::open(&path).unwrap();
            let s = store.ensure_source("a", "A", 0).unwrap();
            store.commit(&[delta(s, 0, 3, 4)], &[]).unwrap();
        }

        // Act
        let store = Store::open(&path).unwrap();

        // Assert
        assert_eq!(store.sum(Range { start: 0, end: B }, None).unwrap(), Bytes::new(3, 4));
    }
}
