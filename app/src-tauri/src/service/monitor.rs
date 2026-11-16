//! The background service: keeps finding cores, reads each one's `/traffic`, accumulates the
//! traffic in memory and writes it to the database once a minute (and on exit), and answers the
//! interface's questions.

use super::discover::{self, Discovery, Found};
use crate::domain::counter::{self, Bytes, Checkpoint, CoreId};
use crate::domain::period::{self, bucket_of, Period, Range};
use crate::infra::mihomo::Client;
use crate::infra::store::{BucketDelta, Store};
use crate::infra::{paths, settings};
use jiff::civil::Weekday;
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
    /// The last 30 days, oldest first.
    pub days: Vec<DayPoint>,
    /// Today, hour by hour.
    pub hours: Vec<HourPoint>,
}

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
        let days = period::last_days(now, 30).map_err(|e| e.to_string())?;
        let hours = period::hours_of_today(now).map_err(|e| e.to_string())?;
        // One read covers every view: from the earliest start to the latest end.
        let start = [day.start, week.start, month.start, year.start, days[0].1.start].into_iter().min().unwrap_or(day.start);
        let end = [day.end, week.end, month.end, year.end].into_iter().max().unwrap_or(day.end);
        let rows = self.buckets(Range { start, end }, source)?;
        let sum = |r: Range| rows.iter().filter(|(b, _)| r.contains(*b)).fold(Bytes::ZERO, |acc, (_, v)| acc + *v);
        Ok(Overview {
            today: sum(day),
            week: sum(week),
            month: sum(month),
            year: sum(year),
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
