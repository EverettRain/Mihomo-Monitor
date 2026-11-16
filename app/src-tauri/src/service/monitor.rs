//! The background service: keeps finding cores, reads each one's `/traffic`, accumulates the
//! traffic in memory and writes it to the database once a minute (and on exit), and answers the
//! interface's questions.

use super::discover::{self, Discovery, Found};
use crate::domain::counter::{self, Bytes, Checkpoint, CoreId};
use crate::domain::period::{self, bucket_of, Period, Range};
use crate::infra::mihomo::Client;
use crate::infra::store::{BucketDelta, Store};
use crate::infra::{paths, settings};
use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::ZonedDateTime;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::Notify;

/// Seconds of per-second rates kept for the live chart.
const LIVE_SECS: usize = 600;
pub const FLUSH_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Connecting,
    Online,
    Offline,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    /// Database id; `None` for a core we found but never managed to read.
    pub id: Option<i64>,
    pub key: String,
    pub name: String,
    pub endpoint: Option<String>,
    pub status: Status,
    pub message: Option<String>,
    pub version: Option<String>,
    pub manual: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LivePoint {
    pub source: i64,
    pub at: i64,
    pub up: u64,
    pub down: u64,
}

pub enum Event {
    /// A per-second rate reading.
    Sample(LivePoint),
    /// The list of sources or their state changed.
    SourcesChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayPoint {
    pub date: String,
    pub up: u64,
    pub down: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourPoint {
    /// Unix seconds of the hour's start.
    pub start: i64,
    pub up: u64,
    pub down: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub today: Bytes,
    pub week: Bytes,
    pub month: Bytes,
    pub year: Bytes,
    /// Everything ever recorded.
    pub all_time: Bytes,
    /// The last 53 weeks day by day (enough for a year heatmap), oldest first.
    pub days: Vec<DayPoint>,
    /// Today, hour by hour.
    pub hours: Vec<HourPoint>,
    /// Today and this month of every recorded source, whatever `source` the overview is for.
    pub by_source: Vec<SourceTotals>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTotals {
    pub id: i64,
    pub today: Bytes,
    pub month: Bytes,
}

/// Sums of contiguous bucket ranges in O(log n), over buckets sorted by time.
struct Prefix {
    buckets: Vec<i64>,
    /// `sums[i]` is the total of the first `i` buckets.
    sums: Vec<Bytes>,
}

impl Prefix {
    fn new(mut rows: Vec<(i64, Bytes)>) -> Prefix {
        rows.sort_by_key(|(b, _)| *b);
        let mut sums = Vec::with_capacity(rows.len() + 1);
        sums.push(Bytes::ZERO);
        for (_, v) in &rows {
            sums.push(*sums.last().unwrap_or(&Bytes::ZERO) + *v);
        }
        Prefix { buckets: rows.into_iter().map(|(b, _)| b).collect(), sums }
    }

    fn sum(&self, r: Range) -> Bytes {
        let lo = self.buckets.partition_point(|&b| b < r.start);
        let hi = self.buckets.partition_point(|&b| b < r.end);
        let (a, b) = (self.sums[lo], self.sums[hi.max(lo)]);
        Bytes { up: b.up - a.up, down: b.down - a.down }
    }
}

/// One period of a history view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodRow {
    /// Unix seconds.
    pub start: i64,
    pub end: i64,
    pub label: String,
    pub up: u64,
    pub down: u64,
}

/// Local dates holding the first and the last recorded traffic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub first: String,
    pub last: String,
}

/// Hour views read at most this many days, so a mistaken range cannot ask for a decade of hours.
const MAX_HOUR_DAYS: i32 = 62;

fn label(kind: Period, start: &ZonedDateTime, end: i64) -> String {
    match kind {
        Period::Hour => start.strftime("%H:00").to_string(),
        Period::Day => start.strftime("%Y-%m-%d").to_string(),
        Period::Week => {
            let last = jiff::Timestamp::from_second(end - 1).map(|t| t.to_zoned_datetime(start.time_zone().clone()));
            match last {
                Ok(l) => format!("{} – {}", start.strftime("%Y-%m-%d"), l.strftime("%m-%d")),
                Err(_) => start.strftime("%Y-%m-%d").to_string(),
            }
        }
        Period::Month => start.strftime("%Y-%m").to_string(),
        Period::Year => start.strftime("%Y").to_string(),
    }
}

fn week_start(sunday: bool) -> Weekday {
    if sunday {
        Weekday::Sunday
    } else {
        Weekday::Monday
    }
}

/// Days in the year heatmap: 53 whole weeks.
const HEATMAP_DAYS: usize = 53 * 7;

/// One reading of a core's `/traffic`, at a unix second.
#[derive(Clone, Copy, Debug)]
struct Reading {
    totals: Bytes,
    rate: Bytes,
    at: i64,
}

#[derive(Default)]
struct State {
    /// Traffic not yet in the database, per (source, bucket).
    pending: BTreeMap<(i64, i64), Bytes>,
    /// Latest counters accounted for, per source; written with `pending`.
    checkpoints: HashMap<i64, Checkpoint>,
    dirty: HashSet<i64>,
    sources: BTreeMap<String, SourceView>,
    /// Keys with a collector reading them.
    running: HashSet<String>,
    /// Running keys no longer wanted (auto-discovery turned off, manual entry removed); their
    /// collectors stop at the next reading.
    stopping: HashSet<String>,
    live: HashMap<i64, VecDeque<LivePoint>>,
}

pub struct Monitor {
    store: Mutex<Store>,
    state: Mutex<State>,
    sink: Box<dyn Fn(Event) + Send + Sync>,
    /// Runs discovery again right away (a core went away, the user added one).
    wake: Notify,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Monitor {
    pub fn new(store: Store, sink: impl Fn(Event) + Send + Sync + 'static) -> Arc<Monitor> {
        let mut state = State::default();
        // Sources recorded before stay listed (and selectable) while their core is not running.
        for s in store.sources().unwrap_or_default() {
            state.sources.insert(
                s.key.clone(),
                SourceView {
                    id: Some(s.id),
                    manual: s.key.starts_with("manual:"),
                    key: s.key,
                    name: s.name,
                    endpoint: None,
                    status: Status::Offline,
                    message: Some("没有发现正在运行的内核".into()),
                    version: None,
                },
            );
        }
        Arc::new(Monitor { store: Mutex::new(store), state: Mutex::new(state), sink: Box::new(sink), wake: Notify::new() })
    }

    pub fn rediscover(&self) {
        self.wake.notify_one();
    }

    /// Finds cores forever: soon while none is being read, then once a minute to notice new ones.
    pub async fn supervise(self: Arc<Self>) {
        loop {
            let s = settings::load();
            let found = discover::discover(&s.manual_sources, s.auto_discover).await;
            self.apply(found, &s);
            let idle = lock(&self.state).running.is_empty();
            let wait = if idle { Duration::from_secs(10) } else { Duration::from_secs(60) };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = self.wake.notified() => {}
            }
        }
    }

    pub async fn flush_forever(self: Arc<Self>) {
        loop {
            tokio::time::sleep(FLUSH_EVERY).await;
            if let Err(e) = self.flush() {
                paths::log(&format!("写入数据库失败：{e}"));
            }
        }
    }

    fn apply(self: &Arc<Self>, d: Discovery, s: &settings::Settings) {
        let manual_keys: HashSet<String> = s.manual_sources.iter().map(|m| discover::manual_key(&m.url)).collect();
        let wanted = |key: &str, manual: bool| if manual { manual_keys.contains(key) } else { s.auto_discover };
        let mut start = Vec::new();
        {
            let mut st = lock(&self.state);
            // A removed manual source goes away, unless it has history worth keeping.
            st.sources.retain(|k, v| !v.manual || manual_keys.contains(k) || v.id.is_some());
            let unwanted: Vec<String> =
                st.sources.values().filter(|v| st.running.contains(&v.key) && !wanted(&v.key, v.manual)).map(|v| v.key.clone()).collect();
            st.stopping.extend(unwanted);
            if !s.auto_discover {
                for v in st.sources.values_mut().filter(|v| !v.manual) {
                    v.status = Status::Offline;
                    v.message = Some("已关闭自动发现本机内核".into());
                }
            }
            for u in d.unreadable {
                if st.running.contains(&u.key) {
                    continue;
                }
                let id = st.sources.get(&u.key).and_then(|v| v.id);
                st.sources.insert(
                    u.key.clone(),
                    SourceView {
                        id,
                        key: u.key,
                        name: u.name,
                        endpoint: u.endpoint,
                        status: Status::Offline,
                        message: Some(u.reason),
                        version: None,
                        manual: u.manual,
                    },
                );
            }
            for f in d.found {
                st.stopping.remove(&f.key);
                if st.running.insert(f.key.clone()) {
                    start.push(f);
                }
            }
        }
        for f in start {
            let id = match lock(&self.store).ensure_source(&f.key, &f.name, paths::now_secs()) {
                Ok(id) => id,
                Err(e) => {
                    paths::log(&e);
                    lock(&self.state).running.remove(&f.key);
                    continue;
                }
            };
            lock(&self.state).sources.insert(
                f.key.clone(),
                SourceView {
                    id: Some(id),
                    key: f.key.clone(),
                    name: f.name.clone(),
                    endpoint: Some(f.endpoint.describe()),
                    status: Status::Connecting,
                    message: None,
                    version: Some(f.version.clone()),
                    manual: f.manual,
                },
            );
            tokio::spawn(self.clone().collect(f, id));
        }
        (self.sink)(Event::SourcesChanged);
    }

    async fn collect(self: Arc<Self>, f: Found, id: i64) {
        let result = self.read(&f, id).await;
        {
            let mut st = lock(&self.state);
            st.running.remove(&f.key);
            if let Some(v) = st.sources.get_mut(&f.key) {
                v.status = Status::Offline;
                v.message = Some(result.err().unwrap_or_else(|| "已停止读取".into()));
            }
        }
        (self.sink)(Event::SourcesChanged);
        self.wake.notify_one();
    }

    async fn read(&self, f: &Found, id: i64) -> Result<(), String> {
        let client = Client::new(&f.endpoint).map_err(|e| e.to_string())?;
        let mut stream = client.traffic().await.map_err(|e| e.to_string())?;
        self.set_status(&f.key, Status::Online, None);
        let mut prev: Option<Bytes> = None;
        while let Some(sample) = stream.next().await.map_err(|e| e.to_string())? {
            let totals = sample.totals.ok_or("内核版本过旧：/traffic 没有累计流量（upTotal / downTotal），请升级 mihomo")?;
            let reading = Reading { totals, rate: sample.rate, at: paths::now_secs() };
            let Some(point) = self.record(&f.key, id, f.core.as_ref(), prev, reading) else {
                return Ok(());
            };
            prev = Some(totals);
            (self.sink)(Event::Sample(point));
        }
        Err("内核关闭了连接".into())
    }

    /// Accounts one reading of a source's counters. `prev` is the previous reading on the same
    /// connection; without one the reading is checked against the last checkpoint. Returns
    /// `None` when the source is no longer wanted.
    fn record(&self, key: &str, id: i64, core: Option<&CoreId>, prev: Option<Bytes>, r: Reading) -> Option<LivePoint> {
        let Reading { totals, rate, at: now } = r;
        let saved = if prev.is_none() && !lock(&self.state).checkpoints.contains_key(&id) {
            lock(&self.store).checkpoint(id).ok().flatten()
        } else {
            None
        };
        let mut st = lock(&self.state);
        if st.stopping.remove(key) || !st.sources.contains_key(key) {
            return None;
        }
        let delta = match prev {
            Some(p) => counter::step(p, totals),
            None => counter::resume(st.checkpoints.get(&id).or(saved.as_ref()), core, totals),
        };
        if !delta.is_zero() {
            *st.pending.entry((id, bucket_of(now))).or_default() += delta;
        }
        st.checkpoints.insert(id, Checkpoint { core: core.cloned(), totals, at: now });
        st.dirty.insert(id);
        let point = LivePoint { source: id, at: now, up: rate.up, down: rate.down };
        let live = st.live.entry(id).or_default();
        live.push_back(point);
        while live.len() > LIVE_SECS {
            live.pop_front();
        }
        Some(point)
    }

    fn set_status(&self, key: &str, status: Status, message: Option<String>) {
        if let Some(v) = lock(&self.state).sources.get_mut(key) {
            v.status = status;
            v.message = message;
        }
        (self.sink)(Event::SourcesChanged);
    }

    /// Writes the accumulated traffic and checkpoints. On failure they stay in memory for the
    /// next attempt.
    pub fn flush(&self) -> Result<(), String> {
        let (pending, dirty) = {
            let mut st = lock(&self.state);
            let dirty: Vec<(i64, Checkpoint)> = st
                .dirty
                .drain()
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|id| st.checkpoints.get(&id).map(|c| (id, c.clone())))
                .collect();
            (std::mem::take(&mut st.pending), dirty)
        };
        if pending.is_empty() && dirty.is_empty() {
            return Ok(());
        }
        let deltas: Vec<BucketDelta> = pending.iter().map(|(&(source, bucket), &bytes)| BucketDelta { source, bucket, bytes }).collect();
        let result = lock(&self.store).commit(&deltas, &dirty);
        if result.is_err() {
            let mut st = lock(&self.state);
            for (k, v) in pending {
                *st.pending.entry(k).or_default() += v;
            }
            st.dirty.extend(dirty.into_iter().map(|(id, _)| id));
        }
        result
    }

    pub fn sources(&self) -> Vec<SourceView> {
        lock(&self.state).sources.values().cloned().collect()
    }

    /// Recent per-second rates of one source, or of every source.
    pub fn live(&self, source: Option<i64>) -> Vec<LivePoint> {
        let st = lock(&self.state);
        st.live.iter().filter(|(id, _)| source.is_none_or(|s| s == **id)).flat_map(|(_, v)| v.iter().copied()).collect()
    }

    /// Traffic per source in `range`: what is in the database plus what is still pending.
    fn sums_by_source(&self, range: Range) -> Result<Vec<(i64, Bytes)>, String> {
        let pending: Vec<(i64, Bytes)> =
            lock(&self.state).pending.iter().filter(|((_, b), _)| range.contains(*b)).map(|((s, _), v)| (*s, *v)).collect();
        let mut out = lock(&self.store).sums_by_source(range)?;
        for (s, v) in pending {
            match out.iter_mut().find(|(id, _)| *id == s) {
                Some((_, total)) => *total += v,
                None => out.push((s, v)),
            }
        }
        Ok(out)
    }

    /// Traffic per bucket in `range`: what is in the database plus what is still pending.
    fn buckets(&self, range: Range, source: Option<i64>) -> Result<Vec<(i64, Bytes)>, String> {
        let pending: Vec<(i64, Bytes)> = lock(&self.state)
            .pending
            .iter()
            .filter(|((s, b), _)| source.is_none_or(|x| x == *s) && range.contains(*b))
            .map(|((_, b), v)| (*b, *v))
            .collect();
        let mut rows = lock(&self.store).buckets(range, source)?;
        rows.extend(pending);
        Ok(rows)
    }

    pub fn overview(&self, source: Option<i64>, week_starts_sunday: bool) -> Result<Overview, String> {
        self.overview_at(&ZonedDateTime::now(), source, week_starts_sunday)
    }

    fn overview_at(&self, now: &ZonedDateTime, source: Option<i64>, week_starts_sunday: bool) -> Result<Overview, String> {
        let week_start = if week_starts_sunday { Weekday::Sunday } else { Weekday::Monday };
        let p = |kind| period::current(kind, now, week_start).map_err(|e| e.to_string());
        let (day, week, month, year) = (p(Period::Day)?, p(Period::Week)?, p(Period::Month)?, p(Period::Year)?);
        let days = period::last_days(now, HEATMAP_DAYS).map_err(|e| e.to_string())?;
        let hours = period::hours_of_today(now).map_err(|e| e.to_string())?;
        // One read covers every view: from the earliest start to the latest end.
        let start = [day.start, week.start, month.start, year.start, days[0].1.start].into_iter().min().unwrap_or(day.start);
        let end = [day.end, week.end, month.end, year.end].into_iter().max().unwrap_or(day.end);
        let prefix = Prefix::new(self.buckets(Range { start, end }, source)?);
        let sum = |r: Range| prefix.sum(r);
        let everything = Range { start: i64::MIN, end: i64::MAX };
        Ok(Overview {
            today: sum(day),
            week: sum(week),
            month: sum(month),
            year: sum(year),
            all_time: self
                .sums_by_source(everything)?
                .into_iter()
                .filter(|(id, _)| source.is_none_or(|s| s == *id))
                .fold(Bytes::ZERO, |a, (_, v)| a + v),
            by_source: {
                let (today, month) = (self.sums_by_source(day)?, self.sums_by_source(month)?);
                let of = |list: &[(i64, Bytes)], id: i64| list.iter().find(|(s, _)| *s == id).map(|(_, v)| *v).unwrap_or_default();
                lock(&self.store)
                    .sources()?
                    .into_iter()
                    .map(|s| SourceTotals { id: s.id, today: of(&today, s.id), month: of(&month, s.id) })
                    .collect()
            },
            days: days
                .iter()
                .map(|(d, r)| {
                    let b = sum(*r);
                    DayPoint { date: d.to_string(), up: b.up, down: b.down }
                })
                .collect(),
            hours: hours
                .iter()
                .map(|r| {
                    let b = sum(*r);
                    HourPoint { start: r.start, up: b.up, down: b.down }
                })
                .collect(),
        })
    }

    /// Traffic per period of `kind` over the local days `from` through `to`, oldest first.
    pub fn history(
        &self,
        kind: Period,
        from: Date,
        to: Date,
        source: Option<i64>,
        week_starts_sunday: bool,
    ) -> Result<Vec<PeriodRow>, String> {
        self.history_in(&TimeZone::system(), kind, from, to, source, week_starts_sunday)
    }

    fn history_in(
        &self,
        tz: &TimeZone,
        kind: Period,
        from: Date,
        to: Date,
        source: Option<i64>,
        week_starts_sunday: bool,
    ) -> Result<Vec<PeriodRow>, String> {
        if to < from {
            return Ok(Vec::new());
        }
        if kind == Period::Hour && from.until(to).map_err(|e| e.to_string())?.get_days() > MAX_HOUR_DAYS {
            return Err(format!("按小时查看最多 {MAX_HOUR_DAYS} 天。"));
        }
        let periods = period::periods(kind, from, to, tz, week_start(week_starts_sunday)).map_err(|e| e.to_string())?;
        let (Some(first), Some(last)) = (periods.first(), periods.last()) else { return Ok(Vec::new()) };
        let prefix = Prefix::new(self.buckets(Range { start: first.1.start, end: last.1.end }, source)?);
        Ok(periods
            .iter()
            .map(|(z, r)| {
                let b = prefix.sum(*r);
                PeriodRow { start: r.start, end: r.end, label: label(kind, z, r.end), up: b.up, down: b.down }
            })
            .collect())
    }

    /// The local dates of the first and last recorded traffic; `None` before any.
    pub fn bounds(&self, source: Option<i64>) -> Result<Option<Bounds>, String> {
        let pending: Vec<i64> = lock(&self.state).pending.keys().filter(|(s, _)| source.is_none_or(|x| x == *s)).map(|(_, b)| *b).collect();
        let stored = lock(&self.store).bounds(source)?;
        let lo = stored.map(|(a, _)| a).into_iter().chain(pending.iter().copied()).min();
        let hi = stored.map(|(_, b)| b).into_iter().chain(pending.iter().copied()).max();
        let tz = TimeZone::system();
        let date = |ts: i64| jiff::Timestamp::from_second(ts).map(|t| t.to_zoned_datetime(tz.clone()).date().to_string());
        match (lo, hi) {
            (Some(lo), Some(hi)) => {
                Ok(Some(Bounds { first: date(lo).map_err(|e| e.to_string())?, last: date(hi).map_err(|e| e.to_string())? }))
            }
            _ => Ok(None),
        }
    }

    /// The `n` days with the most traffic, biggest first (vnstat's top list).
    pub fn top_days(&self, source: Option<i64>, n: usize) -> Result<Vec<PeriodRow>, String> {
        let Some(b) = self.bounds(source)? else { return Ok(Vec::new()) };
        let parse = |s: &str| s.parse::<Date>().map_err(|e| e.to_string());
        let mut days = self.history(Period::Day, parse(&b.first)?, parse(&b.last)?, source, false)?;
        days.retain(|d| d.up + d.down > 0);
        days.sort_by_key(|d| std::cmp::Reverse(d.up + d.down));
        days.truncate(n);
        Ok(days)
    }

    /// Today's traffic of every source, for the tray.
    pub fn today(&self) -> Bytes {
        self.overview(None, false).map(|o| o.today).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor_with_source() -> (Arc<Monitor>, i64) {
        let store = Store::open_in_memory();
        let id = store.ensure_source("a", "A", 0).unwrap();
        (Monitor::new(store, |_| {}), id)
    }

    fn noon() -> ZonedDateTime {
        jiff::civil::date(2026, 11, 16).at(12, 0, 0, 0).in_tz("Asia/Shanghai").unwrap()
    }

    #[test]
    fn readings_count_before_and_after_they_are_written() {
        // Arrange
        let (m, id) = monitor_with_source();
        let core = CoreId { pid: 1, started_at: 0 };
        let now = noon().timestamp().as_second();
        let mut prev = None;
        for (i, total) in [Bytes::new(100, 1_000), Bytes::new(150, 1_600), Bytes::new(180, 2_000)].into_iter().enumerate() {
            m.record("a", id, Some(&core), prev, Reading { totals: total, rate: Bytes::ZERO, at: now + i as i64 }).unwrap();
            prev = Some(total);
        }

        // Act
        let before = m.overview_at(&noon(), Some(id), false).unwrap().today;
        m.flush().unwrap();
        let after = m.overview_at(&noon(), Some(id), false).unwrap().today;

        // Assert: the first reading of a new source is a baseline; the rest is counted once.
        assert_eq!(before, Bytes::new(80, 1_000));
        assert_eq!(after, before);
    }

    #[test]
    fn overview_splits_by_source_and_covers_a_year() {
        // Arrange: two sources, one with traffic already written and one still pending.
        let store = Store::open_in_memory();
        let a = store.ensure_source("a", "A", 0).unwrap();
        let b = store.ensure_source("b", "B", 0).unwrap();
        let m = Monitor::new(store, |_| {});
        let now = noon().timestamp().as_second();
        let last_year = now - 400 * 86_400;
        let read = |key: &str, id: i64, totals: Bytes, at: i64, prev: Option<Bytes>| {
            m.record(key, id, None, prev, Reading { totals, rate: Bytes::ZERO, at }).unwrap();
        };
        read("a", a, Bytes::new(0, 0), last_year, None);
        read("a", a, Bytes::new(1, 10), last_year + 1, Some(Bytes::ZERO));
        read("a", a, Bytes::new(3, 30), now, Some(Bytes::new(1, 10)));
        m.flush().unwrap();
        read("b", b, Bytes::new(0, 0), now, None);
        read("b", b, Bytes::new(5, 50), now + 1, Some(Bytes::ZERO));

        // Act
        let all = m.overview_at(&noon(), None, false).unwrap();
        let only_a = m.overview_at(&noon(), Some(a), false).unwrap();

        // Assert
        assert_eq!(all.today, Bytes::new(7, 70));
        assert_eq!(all.all_time, Bytes::new(8, 80));
        assert_eq!(only_a.all_time, Bytes::new(3, 30));
        assert_eq!(all.days.len(), HEATMAP_DAYS);
        assert_eq!(all.days.last().unwrap().down, 70);
        let totals: Vec<_> = all.by_source.iter().map(|t| (t.id, t.today)).collect();
        assert_eq!(totals, vec![(a, Bytes::new(2, 20)), (b, Bytes::new(5, 50))]);
    }

    #[test]
    fn history_sums_each_period_and_top_days_rank_them() {
        // Arrange: three days of traffic in Asia/Shanghai, the middle one the busiest.
        let (m, id) = monitor_with_source();
        let tz = TimeZone::get("Asia/Shanghai").unwrap();
        let at =
            |d: i8, h: i8| jiff::civil::date(2026, 11, d).at(h, 0, 0, 0).to_zoned_datetime(tz.clone()).unwrap().timestamp().as_second();
        let mut prev = None;
        for (i, (ts, total)) in [(at(15, 9), 0), (at(15, 10), 100), (at(16, 23), 1_100), (at(17, 0), 1_150)].into_iter().enumerate() {
            let t = Bytes::new(0, total);
            m.record("a", id, None, if i == 0 { None } else { prev }, Reading { totals: t, rate: Bytes::ZERO, at: ts }).unwrap();
            prev = Some(t);
        }
        let (from, to) = (jiff::civil::date(2026, 11, 15), jiff::civil::date(2026, 11, 17));

        // Act
        let days = m.history_in(&tz, Period::Day, from, to, Some(id), false).unwrap();
        let hours = m.history_in(&tz, Period::Hour, to, to, Some(id), false).unwrap();
        let weeks = m.history_in(&tz, Period::Week, from, to, Some(id), false).unwrap();

        // Assert
        assert_eq!(
            days.iter().map(|d| (d.label.as_str(), d.down)).collect::<Vec<_>>(),
            [("2026-11-15", 100), ("2026-11-16", 1_000), ("2026-11-17", 50)]
        );
        assert_eq!(hours[0], PeriodRow { start: at(17, 0), end: at(17, 1), label: "00:00".into(), up: 0, down: 50 });
        assert_eq!(
            weeks.iter().map(|w| (w.label.as_str(), w.down)).collect::<Vec<_>>(),
            [("2026-11-09 – 11-15", 100), ("2026-11-16 – 11-22", 1_050)]
        );
    }

    #[test]
    fn hour_views_are_limited_in_length() {
        // Arrange
        let (m, _) = monitor_with_source();
        let tz = TimeZone::get("Asia/Shanghai").unwrap();

        // Act
        let result = m.history_in(&tz, Period::Hour, jiff::civil::date(2026, 1, 1), jiff::civil::date(2026, 6, 1), None, false);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn reconnecting_to_the_same_core_picks_up_from_the_checkpoint() {
        // Arrange
        let (m, id) = monitor_with_source();
        let core = CoreId { pid: 1, started_at: 0 };
        let now = noon().timestamp().as_second();
        m.record("a", id, Some(&core), None, Reading { totals: Bytes::new(100, 100), rate: Bytes::ZERO, at: now }).unwrap();
        m.flush().unwrap();

        // Act: a new Monitor over the same database, as after a restart.
        let store = std::mem::replace(&mut *lock(&m.store), Store::open_in_memory());
        let again = Monitor::new(store, |_| {});
        again.record("a", id, Some(&core), None, Reading { totals: Bytes::new(130, 400), rate: Bytes::ZERO, at: now + 60 }).unwrap();

        // Assert
        assert_eq!(again.overview_at(&noon(), Some(id), false).unwrap().today, Bytes::new(30, 300));
    }

    #[test]
    fn an_unwanted_source_stops_being_recorded() {
        // Arrange: a local core being read when auto-discovery is turned off.
        let (m, id) = monitor_with_source();
        lock(&m.state).running.insert("a".into());
        let off = settings::Settings { auto_discover: false, ..settings::Settings::default() };

        // Act
        m.apply(Discovery::default(), &off);
        let point = m.record("a", id, None, None, Reading { totals: Bytes::new(1, 1), rate: Bytes::ZERO, at: 0 });

        // Assert
        assert!(point.is_none());
        assert_eq!(m.sources()[0].status, Status::Offline);
    }

    #[test]
    fn live_keeps_only_the_recent_window() {
        // Arrange
        let (m, id) = monitor_with_source();
        let mut prev = None;

        // Act
        for i in 0..(LIVE_SECS as i64 + 5) {
            let total = Bytes::new(i as u64, i as u64);
            m.record("a", id, None, prev, Reading { totals: total, rate: Bytes::new(1, 1), at: i }).unwrap();
            prev = Some(total);
        }

        // Assert
        let live = m.live(Some(id));
        assert_eq!(live.len(), LIVE_SECS);
        assert_eq!(live.last().unwrap().at, LIVE_SECS as i64 + 4);
    }
}
