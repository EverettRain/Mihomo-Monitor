//! The statistics database (SQLite).
//!
//! Traffic lives in 15-minute buckets per source and dimension, kept forever. A dimension is one
//! way of splitting traffic (outbound node, subscription, process, inbound); the outbound rows of
//! a bucket always add up to its total, with traffic that could not be attributed under the
//! empty outbound key. Schema changes go through `migrate`, keyed by `PRAGMA user_version`.

use crate::domain::attribution::{AttributionRule, Field};
use crate::domain::counter::{Bytes, Checkpoint, CoreId};
use crate::domain::cycle::{CountMode, Reset};
use crate::domain::hostgroup::{Rule, RuleKind};
use crate::domain::period::Range;
use crate::domain::split::{DimKind, Route};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

const SCHEMA_VERSION: i32 = 6;

/// The rows that make up a total, with the route (`?3`) as a filter: all traffic is the outbound
/// rows, which add up to the total; one route is its route rows.
const SCOPE: &str = "((?3 IS NULL AND d.kind = 'outbound') OR (d.kind = 'route' AND d.key = ?3))";

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

const SCHEMA_V2: &str = "
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// One billing cycle per subscription.
const SCHEMA_V3: &str = "
CREATE UNIQUE INDEX cycles_subscription ON cycles (subscription);
";

/// Host groups and their entries. Groups take precedence in the order they were created.
const SCHEMA_V4: &str = "
CREATE TABLE host_groups (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    note TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
CREATE TABLE host_rules (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    UNIQUE (group_id, kind, value)
);
";

/// Rules that name traffic the core could not tie to a process. They apply as connections are
/// read; what was recorded before a rule existed or changed stays as it was.
const SCHEMA_V5: &str = "
CREATE TABLE attribution_rules (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    field TEXT NOT NULL,
    value TEXT NOT NULL,
    priority INTEGER NOT NULL,
    enabled INTEGER NOT NULL
);
";

/// Through a proxy or not, as a dimension of its own. Traffic recorded before it is converted from
/// the outbound rows by mihomo's built-in names (DIRECT, REJECT…, PASS are direct), the only way
/// to tell without the outbound types the core reports now; the unattributed remainder stays
/// without a route.
const SCHEMA_V6: &str = "
INSERT OR IGNORE INTO dims (kind, key) VALUES ('route', 'proxy'), ('route', 'direct'), ('route', '');
INSERT INTO traffic (source_id, bucket, dim_id, up, down)
SELECT t.source_id, t.bucket, r.id, SUM(t.up), SUM(t.down)
FROM traffic t
JOIN dims o ON o.id = t.dim_id AND o.kind = 'outbound'
JOIN dims r ON r.kind = 'route' AND r.key = CASE
    WHEN o.key = '' THEN ''
    WHEN o.key = 'DIRECT' OR o.key = 'PASS' OR o.key LIKE 'REJECT%' THEN 'direct'
    ELSE 'proxy' END
GROUP BY t.source_id, t.bucket, r.id;
";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRuleRow {
    pub id: i64,
    #[serde(flatten)]
    pub rule: Rule,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostGroupRow {
    pub id: i64,
    pub name: String,
    pub note: String,
    pub rules: Vec<HostRuleRow>,
}

/// What an import or an addition changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Added {
    pub groups: usize,
    pub rules: usize,
    /// Entries the group already had.
    pub duplicates: usize,
}

fn unique_name_err(e: rusqlite::Error, name: &str) -> String {
    match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => format!("已经有名为“{name}”的分组。"),
        e => err(e),
    }
}

/// A subscription's billing cycle as configured.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleRow {
    /// `None` for one not saved yet.
    pub id: Option<i64>,
    pub subscription: String,
    pub reset: Reset,
    /// Allowance per cycle in bytes, when there is one.
    pub quota: Option<u64>,
    pub mode: CountMode,
    /// Percentages of the quota to notify at.
    pub alerts: Vec<u8>,
}

/// Dimension kind of hosts in `domain_daily` (not a traffic dimension).
const HOST: &str = "domain";
/// `domain_daily` up to (not including) this day has been cut down to its top hosts.
const HOSTS_PRUNED_BEFORE: &str = "hosts_pruned_before";

/// Traffic to add to one bucket of a source, under one dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BucketDelta {
    pub source: i64,
    pub bucket: i64,
    pub kind: DimKind,
    pub key: String,
    pub bytes: Bytes,
}

/// Traffic to add to one (process, host) pair of a source on a local day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostDelta {
    pub source: i64,
    /// Local date, `YYYY-MM-DD`.
    pub day: String,
    pub process: String,
    pub host: String,
    pub bytes: Bytes,
}

/// A host's traffic, with the process that made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRow {
    pub process: String,
    pub host: String,
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
    /// Ids of dimension rows, by (kind, key).
    dims: HashMap<(String, String), i64>,
}

fn dim_id(conn: &Connection, cache: &mut HashMap<(String, String), i64>, kind: &str, key: &str) -> rusqlite::Result<i64> {
    if let Some(id) = cache.get(&(kind.to_string(), key.to_string())) {
        return Ok(*id);
    }
    let id = conn.query_row(
        "INSERT INTO dims (kind, key) VALUES (?1, ?2) ON CONFLICT (kind, key) DO UPDATE SET key = excluded.key RETURNING id",
        params![kind, key],
        |r| r.get(0),
    )?;
    cache.insert((kind.to_string(), key.to_string()), id);
    Ok(id)
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
        Ok(Store { conn, dims: HashMap::new() })
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
    pub fn commit(&mut self, deltas: &[BucketDelta], hosts: &[HostDelta], checkpoints: &[(i64, Checkpoint)]) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(err)?;
        // Ids are cached only once the transaction holding them commits.
        let mut dims = self.dims.clone();
        {
            let mut add = tx
                .prepare_cached(
                    "INSERT INTO traffic (source_id, bucket, dim_id, up, down) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (source_id, bucket, dim_id) DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
                )
                .map_err(err)?;
            for d in deltas.iter().filter(|d| !d.bytes.is_zero()) {
                let dim = dim_id(&tx, &mut dims, d.kind.as_str(), &d.key).map_err(err)?;
                add.execute(params![d.source, d.bucket, dim, int(d.bytes.up), int(d.bytes.down)]).map_err(err)?;
            }
            let mut host = tx
                .prepare_cached(
                    "INSERT INTO domain_daily (source_id, day, process_dim_id, domain_dim_id, up, down) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (source_id, day, process_dim_id, domain_dim_id) DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
                )
                .map_err(err)?;
            for h in hosts.iter().filter(|h| !h.bytes.is_zero()) {
                let process = dim_id(&tx, &mut dims, DimKind::Process.as_str(), &h.process).map_err(err)?;
                let name = dim_id(&tx, &mut dims, HOST, &h.host).map_err(err)?;
                host.execute(params![h.source, h.day, process, name, int(h.bytes.up), int(h.bytes.down)]).map_err(err)?;
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
        tx.commit().map_err(err)?;
        self.dims = dims;
        Ok(())
    }

    /// Traffic per key of one dimension in `range`, biggest first.
    pub fn ranking(&self, kind: DimKind, range: Range, source: Option<i64>) -> Result<Vec<(String, Bytes)>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT d.key, SUM(t.up), SUM(t.down)
                 FROM traffic t JOIN dims d ON d.id = t.dim_id
                 WHERE d.kind = ?1 AND t.bucket >= ?2 AND t.bucket < ?3 AND (?4 IS NULL OR t.source_id = ?4)
                 GROUP BY d.key ORDER BY SUM(t.up) + SUM(t.down) DESC",
            )
            .map_err(err)?;
        let rows =
            stmt.query_map(params![kind.as_str(), range.start, range.end, source], |r| Ok((r.get(0)?, bytes_at(r, 1)?))).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Hosts and their processes over the local days `from` through `to`, biggest first.
    pub fn hosts(&self, from: &str, to: &str, source: Option<i64>) -> Result<Vec<HostRow>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT p.key, h.key, SUM(x.up), SUM(x.down)
                 FROM domain_daily x JOIN dims p ON p.id = x.process_dim_id JOIN dims h ON h.id = x.domain_dim_id
                 WHERE x.day >= ?1 AND x.day <= ?2 AND (?3 IS NULL OR x.source_id = ?3)
                 GROUP BY p.key, h.key ORDER BY SUM(x.up) + SUM(x.down) DESC",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![from, to, source], |r| Ok(HostRow { process: r.get(0)?, host: r.get(1)?, bytes: bytes_at(r, 2)? }))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Cuts every finished day before `before` down to its `keep` biggest (process, host) pairs,
    /// plus each process's own `per_process` biggest; the rest of a process's hosts become one
    /// "other" host (the empty key), so per-process totals stay exact. Days already cut are
    /// skipped. Returns how many rows were merged away.
    pub fn prune_hosts(&mut self, before: &str, keep: usize, per_process: usize) -> Result<usize, String> {
        let done: String = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [HOSTS_PRUNED_BEFORE], |r| r.get(0))
            .optional()
            .map_err(err)?
            .unwrap_or_default();
        if done.as_str() >= before {
            return Ok(0);
        }
        let tx = self.conn.transaction().map_err(err)?;
        let mut dims = self.dims.clone();
        let other = dim_id(&tx, &mut dims, HOST, "").map_err(err)?;
        tx.execute_batch("DROP TABLE IF EXISTS temp.dropped").map_err(err)?;
        tx.execute(
            "CREATE TEMP TABLE dropped AS
                 SELECT source_id, day, process_dim_id, domain_dim_id, up, down FROM (
                     SELECT *,
                         ROW_NUMBER() OVER (PARTITION BY source_id, day ORDER BY up + down DESC) AS r_all,
                         ROW_NUMBER() OVER (PARTITION BY source_id, day, process_dim_id ORDER BY up + down DESC) AS r_proc
                     FROM domain_daily WHERE day >= ?1 AND day < ?2)
                 WHERE r_all > ?3 AND r_proc > ?4 AND domain_dim_id != ?5",
            params![done, before, keep as i64, per_process as i64, other],
        )
        .map_err(err)?;
        let merged: i64 = tx.query_row("SELECT COUNT(*) FROM temp.dropped", [], |r| r.get(0)).map_err(err)?;
        tx.execute(
            "INSERT INTO domain_daily (source_id, day, process_dim_id, domain_dim_id, up, down)
             SELECT source_id, day, process_dim_id, ?1, SUM(up), SUM(down) FROM temp.dropped GROUP BY source_id, day, process_dim_id
             ON CONFLICT (source_id, day, process_dim_id, domain_dim_id) DO UPDATE SET up = up + excluded.up, down = down + excluded.down",
            [other],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM domain_daily WHERE (source_id, day, process_dim_id, domain_dim_id) IN
             (SELECT source_id, day, process_dim_id, domain_dim_id FROM temp.dropped)",
            [],
        )
        .map_err(err)?;
        tx.execute_batch("DROP TABLE temp.dropped").map_err(err)?;
        tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", params![HOSTS_PRUNED_BEFORE, before]).map_err(err)?;
        tx.commit().map_err(err)?;
        self.dims = dims;
        Ok(merged as usize)
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

    /// Traffic per source in `range`, all of it or only that of one route; sources without
    /// traffic are left out.
    pub fn sums_by_source(&self, range: Range, route: Option<Route>) -> Result<Vec<(i64, Bytes)>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!(
                "SELECT t.source_id, SUM(t.up), SUM(t.down)
                 FROM traffic t JOIN dims d ON d.id = t.dim_id
                 WHERE {SCOPE} AND t.bucket >= ?1 AND t.bucket < ?2
                 GROUP BY t.source_id"
            ))
            .map_err(err)?;
        let rows =
            stmt.query_map(params![range.start, range.end, route.map(Route::key)], |r| Ok((r.get(0)?, bytes_at(r, 1)?))).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Per-bucket traffic in `range`, all of it or only that of one route, oldest first; empty
    /// buckets are left out.
    pub fn buckets(&self, range: Range, source: Option<i64>, route: Option<Route>) -> Result<Vec<(i64, Bytes)>, String> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!(
                "SELECT t.bucket, SUM(t.up), SUM(t.down)
                 FROM traffic t JOIN dims d ON d.id = t.dim_id
                 WHERE {SCOPE} AND t.bucket >= ?1 AND t.bucket < ?2
                   AND (?4 IS NULL OR t.source_id = ?4)
                 GROUP BY t.bucket ORDER BY t.bucket"
            ))
            .map_err(err)?;
        let rows = stmt
            .query_map(params![range.start, range.end, route.map(Route::key), source], |r| Ok((r.get(0)?, bytes_at(r, 1)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }
    pub fn cycles(&self) -> Result<Vec<CycleRow>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, subscription, anchor, quota_bytes, count_mode, alerts FROM cycles ORDER BY subscription")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, subscription, reset, quota, mode, alerts) = row.map_err(err)?;
            // A row this version cannot read (written by a newer one) is skipped, not fatal.
            let (Ok(reset), Ok(mode), Ok(alerts)) =
                (serde_json::from_str(&reset), serde_json::from_value(serde_json::Value::String(mode)), serde_json::from_str(&alerts))
            else {
                continue;
            };
            out.push(CycleRow { id: Some(id), subscription, reset, quota: quota.map(|q| q.max(0) as u64), mode, alerts });
        }
        Ok(out)
    }

    /// Saves a subscription's cycle, replacing the one it had; returns its id.
    pub fn save_cycle(&self, c: &CycleRow) -> Result<i64, String> {
        let reset = serde_json::to_string(&c.reset).map_err(|e| e.to_string())?;
        let mode = match serde_json::to_value(c.mode).map_err(|e| e.to_string())? {
            serde_json::Value::String(m) => m,
            _ => return Err("计量方式无效。".into()),
        };
        let alerts = serde_json::to_string(&c.alerts).map_err(|e| e.to_string())?;
        self.conn
            .query_row(
                "INSERT INTO cycles (subscription, anchor, quota_bytes, count_mode, alerts) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (subscription) DO UPDATE SET anchor = excluded.anchor, quota_bytes = excluded.quota_bytes,
                     count_mode = excluded.count_mode, alerts = excluded.alerts
                 RETURNING id",
                params![c.subscription, reset, c.quota.map(int), mode, alerts],
                |r| r.get(0),
            )
            .map_err(err)
    }

    pub fn delete_cycle(&self, id: i64) -> Result<(), String> {
        self.conn.execute("DELETE FROM cycles WHERE id = ?1", [id]).map(|_| ()).map_err(err)
    }

    /// Every attribution rule, in the order they are tried.
    pub fn attribution_rules(&self) -> Result<Vec<AttributionRule>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, field, value, priority, enabled FROM attribution_rules ORDER BY priority, id")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get(4)?, r.get(5)?))
            })
            .map_err(err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, field, value, priority, enabled) = row.map_err(err)?;
            // A field from a newer version of the app is left alone rather than misread.
            let Some(field) = Field::parse(&field) else { continue };
            out.push(AttributionRule { id: Some(id), name, field, value, priority, enabled });
        }
        Ok(out)
    }

    /// Adds a rule, or replaces the one with its id.
    pub fn save_attribution_rule(&self, r: &AttributionRule) -> Result<i64, String> {
        let p = params![r.name, r.field.as_str(), r.value, r.priority, r.enabled];
        match r.id {
            Some(id) => {
                let changed = self
                    .conn
                    .execute(
                        "UPDATE attribution_rules SET name = ?1, field = ?2, value = ?3, priority = ?4, enabled = ?5 WHERE id = ?6",
                        params![r.name, r.field.as_str(), r.value, r.priority, r.enabled, id],
                    )
                    .map_err(err)?;
                if changed == 0 {
                    return Err("这条规则已被删除。".into());
                }
                Ok(id)
            }
            None => self
                .conn
                .query_row(
                    "INSERT INTO attribution_rules (name, field, value, priority, enabled) VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
                    p,
                    |r| r.get(0),
                )
                .map_err(err),
        }
    }

    pub fn delete_attribution_rule(&self, id: i64) -> Result<(), String> {
        self.conn.execute("DELETE FROM attribution_rules WHERE id = ?1", [id]).map(|_| ()).map_err(err)
    }

    /// Every group with its entries, in order of precedence.
    pub fn host_groups(&self) -> Result<Vec<HostGroupRow>, String> {
        let mut groups: Vec<HostGroupRow> = {
            let mut stmt = self.conn.prepare("SELECT id, name, note FROM host_groups ORDER BY id").map_err(err)?;
            let rows = stmt
                .query_map([], |r| Ok(HostGroupRow { id: r.get(0)?, name: r.get(1)?, note: r.get(2)?, rules: Vec::new() }))
                .map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        let mut stmt = self.conn.prepare("SELECT id, group_id, kind, value FROM host_rules ORDER BY group_id, kind, value").map_err(err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))
            .map_err(err)?;
        for row in rows {
            let (id, group, kind, value) = row.map_err(err)?;
            let (Some(g), Some(kind)) = (groups.iter_mut().find(|g| g.id == group), RuleKind::parse(&kind)) else { continue };
            g.rules.push(HostRuleRow { id, rule: Rule { kind, value } });
        }
        Ok(groups)
    }

    pub fn create_group(&self, name: &str, note: &str, now: i64) -> Result<i64, String> {
        self.conn
            .query_row("INSERT INTO host_groups (name, note, created_at) VALUES (?1, ?2, ?3) RETURNING id", params![name, note, now], |r| {
                r.get(0)
            })
            .map_err(|e| unique_name_err(e, name))
    }

    pub fn update_group(&self, id: i64, name: &str, note: &str) -> Result<(), String> {
        self.conn
            .execute("UPDATE host_groups SET name = ?2, note = ?3 WHERE id = ?1", params![id, name, note])
            .map(|_| ())
            .map_err(|e| unique_name_err(e, name))
    }

    pub fn delete_group(&mut self, id: i64) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(err)?;
        tx.execute("DELETE FROM host_rules WHERE group_id = ?1", [id]).map_err(err)?;
        tx.execute("DELETE FROM host_groups WHERE id = ?1", [id]).map_err(err)?;
        tx.commit().map_err(err)
    }

    /// Adds entries to a group; ones it already has are counted, not repeated.
    pub fn add_rules(&mut self, group: i64, rules: &[Rule]) -> Result<Added, String> {
        let tx = self.conn.transaction().map_err(err)?;
        let added = insert_rules(&tx, group, rules)?;
        tx.commit().map_err(err)?;
        Ok(added)
    }

    pub fn update_rule(&self, id: i64, rule: &Rule) -> Result<(), String> {
        self.conn
            .execute("UPDATE host_rules SET kind = ?2, value = ?3 WHERE id = ?1", params![id, rule.kind.as_str(), rule.value])
            .map(|_| ())
            .map_err(|e| match e {
                rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
                    "这个分组里已经有相同的条目。".to_string()
                }
                e => err(e),
            })
    }

    pub fn delete_rules(&mut self, ids: &[i64]) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(err)?;
        for id in ids {
            tx.execute("DELETE FROM host_rules WHERE id = ?1", [id]).map_err(err)?;
        }
        tx.commit().map_err(err)
    }

    /// Adds `(group name, entry)` pairs in one transaction, creating groups that do not exist.
    pub fn import_rules(&mut self, entries: &[(String, Rule)], now: i64) -> Result<Added, String> {
        let tx = self.conn.transaction().map_err(err)?;
        let mut total = Added::default();
        let mut by_group: Vec<(String, Vec<Rule>)> = Vec::new();
        for (g, r) in entries {
            match by_group.iter_mut().find(|(name, _)| name == g) {
                Some((_, list)) => list.push(r.clone()),
                None => by_group.push((g.clone(), vec![r.clone()])),
            }
        }
        for (name, rules) in by_group {
            let existing: Option<i64> =
                tx.query_row("SELECT id FROM host_groups WHERE name = ?1", [&name], |r| r.get(0)).optional().map_err(err)?;
            let id = match existing {
                Some(id) => id,
                None => {
                    total.groups += 1;
                    tx.query_row(
                        "INSERT INTO host_groups (name, note, created_at) VALUES (?1, '', ?2) RETURNING id",
                        params![name, now],
                        |r| r.get(0),
                    )
                    .map_err(err)?
                }
            };
            let added = insert_rules(&tx, id, &rules)?;
            total.rules += added.rules;
            total.duplicates += added.duplicates;
        }
        tx.commit().map_err(err)?;
        Ok(total)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>, String> {
        self.conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional().map_err(err)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", params![key, value]).map(|_| ()).map_err(err)
    }
}

fn insert_rules(tx: &rusqlite::Transaction, group: i64, rules: &[Rule]) -> Result<Added, String> {
    let mut stmt = tx.prepare_cached("INSERT OR IGNORE INTO host_rules (group_id, kind, value) VALUES (?1, ?2, ?3)").map_err(err)?;
    let mut added = Added::default();
    for r in rules {
        if stmt.execute(params![group, r.kind.as_str(), r.value]).map_err(err)? == 1 {
            added.rules += 1;
        } else {
            added.duplicates += 1;
        }
    }
    Ok(added)
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(err)?;
    if version > SCHEMA_VERSION {
        return Err("数据库来自更新版本的 Mihomo-Monitor，请升级后再打开。".into());
    }
    if version < 1 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;")).map_err(err)?;
    }
    if version < 2 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V2} PRAGMA user_version = 2; COMMIT;")).map_err(err)?;
    }
    if version < 3 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V3} PRAGMA user_version = 3; COMMIT;")).map_err(err)?;
    }
    if version < 4 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V4} PRAGMA user_version = 4; COMMIT;")).map_err(err)?;
    }
    if version < 5 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V5} PRAGMA user_version = 5; COMMIT;")).map_err(err)?;
    }
    if version < 6 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V6} PRAGMA user_version = 6; COMMIT;")).map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: i64 = crate::domain::period::BUCKET_SECS;

    fn delta(source: i64, bucket: i64, up: u64, down: u64) -> BucketDelta {
        BucketDelta { source, bucket, kind: DimKind::Outbound, key: String::new(), bytes: Bytes::new(up, down) }
    }

    fn dim(source: i64, kind: DimKind, key: &str, down: u64) -> BucketDelta {
        BucketDelta { source, bucket: 0, kind, key: key.into(), bytes: Bytes::new(0, down) }
    }

    fn host(day: &str, process: &str, host: &str, down: u64) -> HostDelta {
        HostDelta { source: 1, day: day.into(), process: process.into(), host: host.into(), bytes: Bytes::new(0, down) }
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
    fn traffic_from_before_routes_is_given_one_by_its_outbound() {
        // Arrange: outbound rows only, as written before the route dimension existed.
        let mut store = Store::open_in_memory();
        let s = store.ensure_source("a", "A", 0).unwrap();
        let out =
            |key: &str, down| BucketDelta { source: s, bucket: 0, kind: DimKind::Outbound, key: key.into(), bytes: Bytes::new(0, down) };
        store.commit(&[out("HK 01", 600), out("DIRECT", 200), out("REJECT-DROP", 5), out("", 100), out("JP 02", 50)], &[], &[]).unwrap();
        let all = Range { start: 0, end: B };

        // Act
        store.conn.execute_batch(SCHEMA_V6).unwrap();
        let routes = store.ranking(DimKind::Route, all, None).unwrap();
        let proxied = store.buckets(all, None, Some(Route::Proxy)).unwrap();
        let total = store.buckets(all, None, None).unwrap();

        // Assert
        assert_eq!(routes.iter().map(|(k, b)| (k.as_str(), b.down)).collect::<Vec<_>>(), [("proxy", 650), ("direct", 205), ("", 100)]);
        assert_eq!(proxied, vec![(0, Bytes::new(0, 650))]);
        assert_eq!(total, vec![(0, Bytes::new(0, 955))]);
    }

    #[test]
    fn commits_add_up_within_a_bucket() {
        // Arrange
        let mut store = Store::open_in_memory();
        let s = store.ensure_source("a", "A", 0).unwrap();

        // Act
        store.commit(&[delta(s, 0, 10, 100)], &[], &[]).unwrap();
        store.commit(&[delta(s, 0, 5, 50), delta(s, B, 1, 1)], &[], &[]).unwrap();

        // Assert
        assert_eq!(
            store.buckets(Range { start: 0, end: 2 * B }, Some(s), None).unwrap(),
            vec![(0, Bytes::new(15, 150)), (B, Bytes::new(1, 1))]
        );
    }

    #[test]
    fn sum_respects_the_range_and_the_source() {
        // Arrange
        let mut store = Store::open_in_memory();
        let a = store.ensure_source("a", "A", 0).unwrap();
        let b = store.ensure_source("b", "B", 0).unwrap();
        store.commit(&[delta(a, 0, 1, 2), delta(a, B, 10, 20), delta(b, B, 100, 200), delta(a, 2 * B, 1000, 2000)], &[], &[]).unwrap();
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
        store.commit(&[], &[], &[(local, with_core.clone()), (remote, without.clone())]).unwrap();

        // Assert
        assert_eq!(store.checkpoint(local).unwrap(), Some(with_core));
        assert_eq!(store.checkpoint(remote).unwrap(), Some(without));
        assert_eq!(store.checkpoint(999).unwrap(), None);
    }

    #[test]
    fn rankings_are_per_kind_and_biggest_first() {
        // Arrange
        let mut store = Store::open_in_memory();
        let s = store.ensure_source("a", "A", 0).unwrap();
        let rows = [dim(s, DimKind::Outbound, "HK", 5), dim(s, DimKind::Outbound, "DIRECT", 9), dim(s, DimKind::Process, "Safari", 4)];
        store.commit(&rows, &[], &[]).unwrap();
        store.commit(&[dim(s, DimKind::Outbound, "HK", 6)], &[], &[]).unwrap();

        // Act
        let outbound = store.ranking(DimKind::Outbound, Range { start: 0, end: B }, None).unwrap();
        let process = store.ranking(DimKind::Process, Range { start: 0, end: B }, Some(s)).unwrap();

        // Assert
        assert_eq!(outbound, vec![("HK".to_string(), Bytes::new(0, 11)), ("DIRECT".to_string(), Bytes::new(0, 9))]);
        assert_eq!(process, vec![("Safari".to_string(), Bytes::new(0, 4))]);
        assert_eq!(store.sum(Range { start: 0, end: B }, None).unwrap(), Bytes::new(0, 20));
    }

    #[test]
    fn pruning_keeps_the_top_hosts_and_folds_the_rest_per_process() {
        // Arrange: yesterday had five hosts, today one.
        let mut store = Store::open_in_memory();
        store.ensure_source("a", "A", 0).unwrap();
        let hosts = [
            host("2026-11-15", "Chrome", "a.com", 50),
            host("2026-11-15", "Chrome", "b.com", 40),
            host("2026-11-15", "Chrome", "c.com", 3),
            host("2026-11-15", "Mail", "m.com", 2),
            host("2026-11-15", "Mail", "n.com", 1),
            host("2026-11-16", "Chrome", "z.com", 1),
        ];
        store.commit(&[], &hosts, &[]).unwrap();

        // Act: keep 2 overall, and each process's single biggest.
        let merged = store.prune_hosts("2026-11-16", 2, 1).unwrap();
        let again = store.prune_hosts("2026-11-16", 2, 1).unwrap();

        // Assert
        let day = |d: &str| {
            let mut v: Vec<_> = store.hosts(d, d, None).unwrap().into_iter().map(|h| (h.process, h.host, h.bytes.down)).collect();
            v.sort();
            v
        };
        assert_eq!((merged, again), (2, 0));
        assert_eq!(
            day("2026-11-15"),
            vec![
                ("Chrome".into(), "".into(), 3),
                ("Chrome".into(), "a.com".into(), 50),
                ("Chrome".into(), "b.com".into(), 40),
                ("Mail".into(), "".into(), 1),
                ("Mail".into(), "m.com".into(), 2),
            ]
        );
        assert_eq!(day("2026-11-16").len(), 1);
    }

    #[test]
    fn a_subscription_has_one_cycle_which_saving_replaces() {
        // Arrange
        let store = Store::open_in_memory();
        let mut c = CycleRow {
            id: None,
            subscription: "白月光".into(),
            reset: Reset::Monthly { day: 15, hour: 0, minute: 0 },
            quota: Some(200 << 30),
            mode: CountMode::Both,
            alerts: vec![80, 100],
        };

        // Act
        let id = store.save_cycle(&c).unwrap();
        c.mode = CountMode::DownloadOnly;
        c.quota = None;
        let again = store.save_cycle(&c).unwrap();
        let saved = store.cycles().unwrap();
        store.delete_cycle(id).unwrap();

        // Assert
        assert_eq!(id, again);
        assert_eq!(saved, vec![CycleRow { id: Some(id), ..c }]);
        assert!(store.cycles().unwrap().is_empty());
    }

    #[test]
    fn attribution_rules_are_saved_listed_in_order_and_deleted() {
        // Arrange
        let store = Store::open_in_memory();
        let rule = |name: &str, priority| AttributionRule {
            id: None,
            name: name.into(),
            field: Field::Inbound,
            value: "US-Workstation-Port".into(),
            priority,
            enabled: true,
        };
        let later = store.save_attribution_rule(&rule("后试", 20)).unwrap();
        let first = store.save_attribution_rule(&rule("先试", 10)).unwrap();

        // Act
        store.save_attribution_rule(&AttributionRule { id: Some(later), enabled: false, ..rule("改名", 20) }).unwrap();
        let listed = store.attribution_rules().unwrap();
        store.delete_attribution_rule(first).unwrap();
        let gone = store.save_attribution_rule(&AttributionRule { id: Some(first), ..rule("先试", 10) });

        // Assert
        assert_eq!(listed.iter().map(|r| (r.name.as_str(), r.enabled)).collect::<Vec<_>>(), [("先试", true), ("改名", false)]);
        assert_eq!(store.attribution_rules().unwrap().len(), 1);
        assert!(gone.is_err());
    }

    #[test]
    fn host_groups_keep_unique_names_and_entries() {
        // Arrange
        let mut store = Store::open_in_memory();
        let yt = store.create_group("YouTube", "", 0).unwrap();
        let suffix = |v: &str| Rule::new(RuleKind::Suffix, v).unwrap();

        // Act
        let added = store.add_rules(yt, &[suffix("youtube.com"), suffix("ytimg.com"), suffix("youtube.com")]).unwrap();
        let duplicate_name = store.create_group("YouTube", "", 0);
        let imported = store.import_rules(&[("YouTube".into(), suffix("ytimg.com")), ("Telegram".into(), suffix("t.me"))], 1).unwrap();
        let groups = store.host_groups().unwrap();
        let first = groups[0].rules[0].id;
        let clash = store.update_rule(first, &suffix("ytimg.com"));
        store.delete_group(groups[1].id).unwrap();

        // Assert
        assert_eq!(added, Added { groups: 0, rules: 2, duplicates: 1 });
        assert!(duplicate_name.unwrap_err().contains("YouTube"));
        assert_eq!(imported, Added { groups: 1, rules: 1, duplicates: 1 });
        assert_eq!(groups.iter().map(|g| (g.name.as_str(), g.rules.len())).collect::<Vec<_>>(), [("YouTube", 2), ("Telegram", 1)]);
        assert!(clash.is_err());
        assert_eq!(store.host_groups().unwrap().len(), 1);
    }

    #[test]
    fn reopening_keeps_data_and_schema() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traffic.db");
        {
            let mut store = Store::open(&path).unwrap();
            let s = store.ensure_source("a", "A", 0).unwrap();
            store.commit(&[delta(s, 0, 3, 4)], &[], &[]).unwrap();
        }

        // Act
        let store = Store::open(&path).unwrap();

        // Assert
        assert_eq!(store.sum(Range { start: 0, end: B }, None).unwrap(), Bytes::new(3, 4));
    }
}
