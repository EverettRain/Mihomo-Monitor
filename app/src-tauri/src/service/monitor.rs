//! The background service: keeps finding cores and reads each one. `/traffic` gives the
//! per-second rate for the live chart; `/connections`, polled every two seconds, gives the
//! counters the statistics are built on and, from the open connections, where the traffic went.
//! Traffic accumulates in memory and is written to the database once a minute (and on exit).
//! Also answers the interface's questions.

use super::discover::{self, Discovery, Found};
use crate::domain::attribution::{AttributionRule, Attributor};
use crate::domain::counter::{self, Bytes, Checkpoint, CoreId};
use crate::domain::cycle::{self, Reported};
use crate::domain::hostgroup::{Matcher, Rule, RuleKind};
use crate::domain::period::{self, bucket_of, Period, Range};
use crate::domain::split::{self, DimKey, DimKind, Dims, HostKey, OutboundTypes, Route, Tracker};
use crate::infra::import::{self, Entry, Format, Parsed, Problem};
use crate::infra::mihomo::Client;
use crate::infra::store::{Added, BucketDelta, CycleRow, HostDelta, HostGroupRow, Store};
use crate::infra::{discovery, paths, settings, verge};
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
/// How often the open connections are read. Shorter catches more of the last bytes of
/// connections that close quickly; each read parses the whole connection list.
const POLL_EVERY: Duration = Duration::from_secs(2);
/// A finished day keeps this many (process, host) pairs, and each process its own biggest few.
const HOSTS_KEPT: usize = 200;
const HOSTS_PER_PROCESS: usize = 10;
/// How often providers' usage reports are fetched from a core; they change only when the client
/// refreshes the subscription.
const REPORTS_EVERY: Duration = Duration::from_secs(10 * 60);
/// How soon outbound types are read again when a connection uses an outbound not among them.
const TYPES_RETRY: Duration = Duration::from_secs(60);
/// Subscriptions with traffic within this many days are listed even without a cycle.
const SUBSCRIPTIONS_SEEN_DAYS: i64 = 400;

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
    /// A subscription's use in its cycle reached one of its alert thresholds.
    Alert(Alert),
}

pub struct Alert {
    pub subscription: String,
    /// Percent of the quota reached.
    pub threshold: u8,
    pub used: u64,
    pub quota: u64,
    /// When the cycle resets, unix seconds.
    pub resets_at: i64,
}

/// A subscription's current cycle and its use so far.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleView {
    #[serde(flatten)]
    pub config: CycleRow,
    /// Unix seconds.
    pub start: i64,
    pub end: i64,
    /// Counted the configured way (both directions or download only).
    pub used: u64,
    pub bytes: Bytes,
    /// Use at the end of the cycle at the pace so far.
    pub projected: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionView {
    pub name: String,
    pub cycle: Option<CycleView>,
    /// The provider's own report, when the client or core has one.
    pub reported: Option<Reported>,
    /// This calendar month, for subscriptions without a cycle.
    pub month: Bytes,
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

/// One key of a ranking: a node, subscription, process, inbound or host. The empty key is
/// traffic that could not be attributed (or, for hosts, the many small ones folded together).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankRow {
    pub key: String,
    pub up: u64,
    pub down: u64,
    /// The key is a host group's name rather than a host.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub group: bool,
    /// Traffic the core counted that no connection read accounts for (key empty). Listed apart
    /// for processes only, where the empty key itself is traffic of connections without one.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub uncollected: bool,
    /// In a ranking of one route: traffic of that route recorded, before routes were kept for
    /// this dimension, without one (key empty).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unrouted: bool,
}

/// Whether a row of route `row` is wanted when asking for `route`, or for all traffic.
fn in_route(row: &str, route: Option<Route>) -> bool {
    route.is_none_or(|r| r.key() == row)
}

fn ranked(map: HashMap<String, Bytes>) -> Vec<RankRow> {
    ranked_marked(map.into_iter().map(|(k, b)| ((k, false), b)).collect())
}

fn ranked_marked(map: HashMap<(String, bool), Bytes>) -> Vec<RankRow> {
    let mut rows: Vec<RankRow> = map
        .into_iter()
        .filter(|(_, b)| !b.is_zero())
        .map(|((key, group), b)| RankRow { key, up: b.up, down: b.down, group, uncollected: false, unrouted: false })
        .collect();
    sort_rows(&mut rows);
    rows
}

fn sort_rows(rows: &mut [RankRow]) {
    rows.sort_by(|a, b| {
        (b.up + b.down)
            .cmp(&(a.up + a.down))
            .then_with(|| a.key.cmp(&b.key))
            .then((a.uncollected, a.unrouted).cmp(&(b.uncollected, b.unrouted)))
    });
}

/// How hosts are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostView {
    /// As connected to.
    Host,
    /// Under their registrable domain.
    Domain,
    /// Under their host group's name; hosts in no group under their registrable domain.
    Group,
}

/// What the entries typed or pasted into a group added, and the lines that could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddResult {
    pub added: Added,
    pub problems: Vec<Problem>,
}

fn group_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    match name.chars().count() {
        0 => Err("请填写分组名称。".into()),
        n if n > 60 => Err("分组名称最多 60 个字。".into()),
        _ => Ok(name.to_string()),
    }
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

/// Local date of a unix second, `YYYY-MM-DD`.
fn local_day(ts: i64) -> String {
    jiff::Timestamp::from_second(ts).map(|t| t.to_zoned_datetime(TimeZone::system()).date().to_string()).unwrap_or_default()
}

#[derive(Default)]
struct State {
    /// Traffic not yet in the database, per (source, bucket, dimension key and route). The
    /// outbound entries of a bucket add up to its total.
    pending: BTreeMap<(i64, i64, DimKey), Bytes>,
    /// Host traffic not yet in the database, per (source, local day, process, host and route).
    pending_hosts: BTreeMap<(i64, String, HostKey), Bytes>,
    /// Finished days before this one have had their hosts cut down to the top ones.
    hosts_pruned_before: String,
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
    /// Latest usage report per subscription name.
    reported: HashMap<String, Reported>,
}

pub struct Monitor {
    store: Mutex<Store>,
    state: Mutex<State>,
    sink: Box<dyn Fn(Event) + Send + Sync>,
    /// Runs discovery again right away (a core went away, the user added one).
    wake: Notify,
    /// Host groups compiled for matching; rebuilt after they change.
    matcher: Mutex<Option<Arc<Matcher>>>,
    /// The enabled attribution rules, read once and kept until they change.
    attributor: Mutex<Option<Arc<Attributor>>>,
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
        Arc::new(Monitor {
            store: Mutex::new(store),
            state: Mutex::new(state),
            sink: Box::new(sink),
            wake: Notify::new(),
            matcher: Mutex::new(None),
            attributor: Mutex::new(None),
        })
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
            if let Err(e) = self.check_alerts(paths::now_secs()) {
                paths::log(&format!("检查流量提醒失败：{e}"));
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
            // A core no longer seen at all, and never read, leaves nothing worth listing.
            let present: HashSet<&str> =
                d.found.iter().map(|f| f.key.as_str()).chain(d.unreadable.iter().map(|u| u.key.as_str())).collect();
            let running = st.running.clone();
            st.sources.retain(|k, v| v.manual || v.id.is_some() || present.contains(k.as_str()) || running.contains(k));
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

    /// Reads a core until it fails or is no longer wanted: rates and accounting side by side.
    async fn read(&self, f: &Found, id: i64) -> Result<(), String> {
        let client = Client::new(&f.endpoint).map_err(|e| e.to_string())?;
        tokio::select! {
            r = self.read_rates(&client, f, id) => r,
            r = self.read_connections(&client, f, id) => r,
        }
    }

    async fn read_rates(&self, client: &Client, f: &Found, id: i64) -> Result<(), String> {
        let mut stream = client.traffic().await.map_err(|e| e.to_string())?;
        self.set_status(&f.key, Status::Online, None);
        while let Some(sample) = stream.next().await.map_err(|e| e.to_string())? {
            let Some(point) = self.live_point(&f.key, id, sample.rate, paths::now_secs()) else { return Ok(()) };
            (self.sink)(Event::Sample(point));
        }
        Err("内核关闭了连接".into())
    }

    async fn read_connections(&self, client: &Client, f: &Found, id: i64) -> Result<(), String> {
        let mut prev: Option<Bytes> = None;
        let mut tracker = Tracker::default();
        let mut profiles = (f.key == discovery::Client::Verge.key()).then(verge::Profiles::default);
        let mut tick = tokio::time::interval(POLL_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut reports_due = tokio::time::Instant::now();
        let mut types = OutboundTypes::new();
        let mut types_read: Option<tokio::time::Instant> = None;
        loop {
            tick.tick().await;
            let snap = client.connections().await.map_err(|e| e.to_string())?;
            let now = tokio::time::Instant::now();
            if now >= reports_due {
                reports_due += REPORTS_EVERY;
                // A core without providers, or an older one, simply has nothing to report.
                if let Ok(list) = client.providers().await {
                    self.note_reports(list);
                }
            }
            // Outbound types tell direct traffic from proxied. Read again with the reports, and
            // sooner when a connection uses an outbound we have not seen (the config changed).
            let unseen = snap.conns.iter().filter_map(|c| c.chains.first()).any(|o| !o.is_empty() && !types.contains_key(o));
            let due =
                types_read.is_none_or(|at| now.duration_since(at) >= REPORTS_EVERY || (unseen && now.duration_since(at) >= TYPES_RETRY));
            if due {
                types_read = Some(now);
                // Without them, mihomo's built-in names still tell DIRECT and REJECT apart.
                if let Ok(t) = client.proxy_types().await {
                    types = t;
                }
            }
            let subscription = profiles.as_mut().and_then(|p| {
                p.refresh();
                self.note_reports(p.reported().to_vec());
                p.active().map(str::to_string)
            });
            // Rules as they are now: a change applies from the next reading on.
            let rules = self.attributor()?;
            let deltas: Vec<(Dims, Bytes)> = tracker
                .step(&snap.conns)
                .into_iter()
                .map(|(c, b)| (split::dims_of(c, subscription.as_deref(), &rules, &types), b))
                .collect();
            if !self.account(&f.key, id, f.core.as_ref(), prev, snap.totals, &deltas, paths::now_secs()) {
                return Ok(());
            }
            prev = Some(snap.totals);
        }
    }

    /// Accounts one reading of a source's counters and the growth of its connections since the
    /// last reading. `prev` is the previous reading on the same connection; without one the
    /// reading is checked against the last checkpoint. Returns `false` when the source is no
    /// longer wanted.
    #[allow(clippy::too_many_arguments)]
    fn account(
        &self,
        key: &str,
        id: i64,
        core: Option<&CoreId>,
        prev: Option<Bytes>,
        totals: Bytes,
        conns: &[(Dims, Bytes)],
        now: i64,
    ) -> bool {
        let saved = if prev.is_none() && !lock(&self.state).checkpoints.contains_key(&id) {
            lock(&self.store).checkpoint(id).ok().flatten()
        } else {
            None
        };
        let mut st = lock(&self.state);
        if st.stopping.remove(key) || !st.sources.contains_key(key) {
            return false;
        }
        let delta = match prev {
            Some(p) => counter::step(p, totals),
            None => counter::resume(st.checkpoints.get(&id).or(saved.as_ref()), core, totals),
        };
        if !delta.is_zero() || !conns.is_empty() {
            let s = split::split(delta, conns);
            let bucket = bucket_of(now);
            for (k, b) in s.dims {
                *st.pending.entry((id, bucket, k)).or_default() += b;
            }
            let day = local_day(now);
            for (k, b) in s.hosts {
                *st.pending_hosts.entry((id, day.clone(), k)).or_default() += b;
            }
        }
        st.checkpoints.insert(id, Checkpoint { core: core.cloned(), totals, at: now });
        st.dirty.insert(id);
        true
    }

    fn note_reports(&self, list: Vec<(String, Reported)>) {
        let mut st = lock(&self.state);
        for (name, r) in list {
            st.reported.insert(name, r);
        }
    }

    /// Keeps a per-second rate for the live chart; `None` when the source is no longer wanted.
    fn live_point(&self, key: &str, id: i64, rate: Bytes, now: i64) -> Option<LivePoint> {
        let mut st = lock(&self.state);
        if st.stopping.contains(key) || !st.sources.contains_key(key) {
            return None;
        }
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
        let (pending, hosts, dirty) = {
            let mut st = lock(&self.state);
            let dirty: Vec<(i64, Checkpoint)> = st
                .dirty
                .drain()
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|id| st.checkpoints.get(&id).map(|c| (id, c.clone())))
                .collect();
            (std::mem::take(&mut st.pending), std::mem::take(&mut st.pending_hosts), dirty)
        };
        if !(pending.is_empty() && hosts.is_empty() && dirty.is_empty()) {
            let deltas: Vec<BucketDelta> = pending
                .iter()
                .map(|((source, bucket, k), bytes)| BucketDelta {
                    source: *source,
                    bucket: *bucket,
                    kind: k.kind,
                    key: k.key.clone(),
                    route: k.route.clone(),
                    bytes: *bytes,
                })
                .collect();
            let host_deltas: Vec<HostDelta> = hosts
                .iter()
                .map(|((source, day, k), bytes)| HostDelta {
                    source: *source,
                    day: day.clone(),
                    process: k.process.clone(),
                    host: k.host.clone(),
                    route: k.route.clone(),
                    bytes: *bytes,
                })
                .collect();
            let result = lock(&self.store).commit(&deltas, &host_deltas, &dirty);
            if result.is_err() {
                let mut st = lock(&self.state);
                for (k, v) in pending {
                    *st.pending.entry(k).or_default() += v;
                }
                for (k, v) in hosts {
                    *st.pending_hosts.entry(k).or_default() += v;
                }
                st.dirty.extend(dirty.into_iter().map(|(id, _)| id));
                return result;
            }
        }
        self.prune_finished_days(&local_day(paths::now_secs()))
    }

    /// Once a day: finished days keep only their top hosts (today stays exact).
    fn prune_finished_days(&self, today: &str) -> Result<(), String> {
        if lock(&self.state).hosts_pruned_before.as_str() >= today {
            return Ok(());
        }
        lock(&self.store).prune_hosts(today, HOSTS_KEPT, HOSTS_PER_PROCESS)?;
        lock(&self.state).hosts_pruned_before = today.to_string();
        Ok(())
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
    fn sums_by_source(&self, range: Range, route: Option<Route>) -> Result<Vec<(i64, Bytes)>, String> {
        let pending: Vec<(i64, Bytes)> = lock(&self.state)
            .pending
            .iter()
            .filter(|((_, b, k), _)| k.kind == DimKind::Outbound && in_route(&k.route, route) && range.contains(*b))
            .map(|((s, ..), v)| (*s, *v))
            .collect();
        let mut out = lock(&self.store).sums_by_source(range, route)?;
        for (s, v) in pending {
            match out.iter_mut().find(|(id, _)| *id == s) {
                Some((_, total)) => *total += v,
                None => out.push((s, v)),
            }
        }
        Ok(out)
    }

    /// Traffic per bucket in `range`, all or one route's: what is in the database plus what is
    /// still pending.
    fn buckets(&self, range: Range, source: Option<i64>, route: Option<Route>) -> Result<Vec<(i64, Bytes)>, String> {
        let pending: Vec<(i64, Bytes)> = lock(&self.state)
            .pending
            .iter()
            .filter(|((s, b, k), _)| {
                k.kind == DimKind::Outbound && in_route(&k.route, route) && source.is_none_or(|x| x == *s) && range.contains(*b)
            })
            .map(|((_, b, ..), v)| (*b, *v))
            .collect();
        let mut rows = lock(&self.store).buckets(range, source, route)?;
        rows.extend(pending);
        Ok(rows)
    }

    /// The figures of the overview, of all traffic or of one route's.
    pub fn overview(&self, source: Option<i64>, route: Option<Route>, week_starts_sunday: bool) -> Result<Overview, String> {
        self.overview_at(&ZonedDateTime::now(), source, route, week_starts_sunday)
    }

    fn overview_at(
        &self,
        now: &ZonedDateTime,
        source: Option<i64>,
        route: Option<Route>,
        week_starts_sunday: bool,
    ) -> Result<Overview, String> {
        let week_start = if week_starts_sunday { Weekday::Sunday } else { Weekday::Monday };
        let p = |kind| period::current(kind, now, week_start).map_err(|e| e.to_string());
        let (day, week, month, year) = (p(Period::Day)?, p(Period::Week)?, p(Period::Month)?, p(Period::Year)?);
        let days = period::last_days(now, HEATMAP_DAYS).map_err(|e| e.to_string())?;
        let hours = period::hours_of_today(now).map_err(|e| e.to_string())?;
        // One read covers every view: from the earliest start to the latest end.
        let start = [day.start, week.start, month.start, year.start, days[0].1.start].into_iter().min().unwrap_or(day.start);
        let end = [day.end, week.end, month.end, year.end].into_iter().max().unwrap_or(day.end);
        let prefix = Prefix::new(self.buckets(Range { start, end }, source, route)?);
        let sum = |r: Range| prefix.sum(r);
        let everything = Range { start: i64::MIN, end: i64::MAX };
        Ok(Overview {
            today: sum(day),
            week: sum(week),
            month: sum(month),
            year: sum(year),
            all_time: self
                .sums_by_source(everything, route)?
                .into_iter()
                .filter(|(id, _)| source.is_none_or(|s| s == *id))
                .fold(Bytes::ZERO, |a, (_, v)| a + v),
            by_source: {
                let (today, month) = (self.sums_by_source(day, route)?, self.sums_by_source(month, route)?);
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
        route: Option<Route>,
        week_starts_sunday: bool,
    ) -> Result<Vec<PeriodRow>, String> {
        self.history_in(&TimeZone::system(), kind, from, to, source, route, week_starts_sunday)
    }

    #[allow(clippy::too_many_arguments)]
    fn history_in(
        &self,
        tz: &TimeZone,
        kind: Period,
        from: Date,
        to: Date,
        source: Option<i64>,
        route: Option<Route>,
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
        let prefix = Prefix::new(self.buckets(Range { start: first.1.start, end: last.1.end }, source, route)?);
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
        let pending: Vec<i64> =
            lock(&self.state).pending.keys().filter(|(s, ..)| source.is_none_or(|x| x == *s)).map(|(_, b, ..)| *b).collect();
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

    /// The `n` days with the most traffic (of all or of one route), biggest first (vnstat's top list).
    pub fn top_days(&self, source: Option<i64>, route: Option<Route>, n: usize) -> Result<Vec<PeriodRow>, String> {
        let Some(b) = self.bounds(source)? else { return Ok(Vec::new()) };
        let parse = |s: &str| s.parse::<Date>().map_err(|e| e.to_string());
        let mut days = self.history(Period::Day, parse(&b.first)?, parse(&b.last)?, source, route, false)?;
        days.retain(|d| d.up + d.down > 0);
        days.sort_by_key(|d| std::cmp::Reverse(d.up + d.down));
        days.truncate(n);
        Ok(days)
    }

    /// The time span of the local days `from` through `to`.
    fn days_range(&self, from: Date, to: Date) -> Result<Option<Range>, String> {
        let days = period::periods(Period::Day, from, to, &TimeZone::system(), Weekday::Monday).map_err(|e| e.to_string())?;
        Ok(days.first().zip(days.last()).map(|(a, b)| Range { start: a.1.start, end: b.1.end }))
    }

    /// Traffic per key of one dimension over the local days `from` through `to`, biggest first.
    /// Dimensions other than the outbound one know only part of the traffic (and nothing before
    /// per-connection collection began); the rest is listed under the empty key. For processes
    /// the rest is a row of its own, marked `uncollected`, and the empty key is only the traffic
    /// of connections the core named no process for and no rule matched.
    pub fn ranking(&self, kind: DimKind, from: Date, to: Date, source: Option<i64>, route: Option<Route>) -> Result<Vec<RankRow>, String> {
        let Some(range) = self.days_range(from, to)? else { return Ok(Vec::new()) };
        let mut map = self.kind_sums(kind, range, source, route)?;
        if kind == DimKind::Outbound {
            return Ok(ranked(map));
        }
        let total = Prefix::new(self.buckets(range, source, route)?).sum(range);
        let known = map.values().fold(Bytes::ZERO, |a, b| a + *b);
        let rest = Bytes::new(total.up.saturating_sub(known.up), total.down.saturating_sub(known.down));
        // Within one route, everything was carried by connections read (what none accounts for
        // has no route), so what is missing is rows written without a route: before routes
        // were kept for this dimension.
        let (uncollected, unrouted) = (route.is_none() && kind == DimKind::Process, route.is_some());
        if !(uncollected || unrouted) {
            *map.entry(String::new()).or_default() += rest;
            return Ok(ranked(map));
        }
        let mut rows = ranked(map);
        if !rest.is_zero() {
            rows.push(RankRow { key: String::new(), up: rest.up, down: rest.down, group: false, uncollected, unrouted });
            sort_rows(&mut rows);
        }
        Ok(rows)
    }

    /// Traffic per key of one dimension in `range`, of all traffic or one route's, written and
    /// pending.
    fn kind_sums(&self, kind: DimKind, range: Range, source: Option<i64>, route: Option<Route>) -> Result<HashMap<String, Bytes>, String> {
        let mut map: HashMap<String, Bytes> = HashMap::new();
        for (key, b) in lock(&self.store).ranking(kind, range, source, route)? {
            *map.entry(key).or_default() += b;
        }
        for ((s, bucket, k), b) in lock(&self.state).pending.iter() {
            if k.kind == kind && in_route(&k.route, route) && range.contains(*bucket) && source.is_none_or(|x| x == *s) {
                *map.entry(k.key.clone()).or_default() += *b;
            }
        }
        Ok(map)
    }

    /// Traffic over the local days `from` through `to` by route: `proxy`, `direct`, and the empty
    /// key for what has none. Together they are all the traffic.
    pub fn routes(&self, from: Date, to: Date, source: Option<i64>) -> Result<Vec<RankRow>, String> {
        let Some(range) = self.days_range(from, to)? else { return Ok(Vec::new()) };
        let mut map: HashMap<String, Bytes> = HashMap::new();
        for (route, b) in lock(&self.store).routes(range, source)? {
            *map.entry(route).or_default() += b;
        }
        for ((s, bucket, k), b) in lock(&self.state).pending.iter() {
            if k.kind == DimKind::Outbound && range.contains(*bucket) && source.is_none_or(|x| x == *s) {
                *map.entry(k.route.clone()).or_default() += *b;
            }
        }
        Ok(ranked(map))
    }

    /// Every subscription known — seen in traffic, reported by a client or core, or given a
    /// cycle — with its current cycle, the provider's report and this month's use. Subscriptions
    /// with a cycle come first, then by this month's use.
    pub fn subscriptions(&self, now: i64) -> Result<Vec<SubscriptionView>, String> {
        let tz = TimeZone::system();
        let zoned = jiff::Timestamp::from_second(now).map_err(|e| e.to_string())?.to_zoned_datetime(tz.clone());
        let month = period::current(Period::Month, &zoned, Weekday::Monday).map_err(|e| e.to_string())?;
        let seen = Range { start: now - SUBSCRIPTIONS_SEEN_DAYS * 86_400, end: now + 1 };
        let month_use = self.kind_sums(DimKind::Subscription, month, None, None)?;
        let cycles = lock(&self.store).cycles()?;
        let reported = lock(&self.state).reported.clone();

        let mut names: Vec<String> = self.kind_sums(DimKind::Subscription, seen, None, None)?.into_keys().collect();
        names.extend(reported.keys().cloned());
        names.extend(cycles.iter().map(|c| c.subscription.clone()));
        names.retain(|n| !n.is_empty() && n != split::DIRECT);
        names.sort();
        names.dedup();

        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let cycle = match cycles.iter().find(|c| c.subscription == name) {
                Some(c) => Some(self.cycle_view(c, now, &tz)?),
                None => None,
            };
            out.push(SubscriptionView {
                cycle,
                reported: reported.get(&name).copied(),
                month: month_use.get(&name).copied().unwrap_or_default(),
                name,
            });
        }
        out.sort_by(|a, b| {
            b.cycle
                .is_some()
                .cmp(&a.cycle.is_some())
                .then((b.month.up + b.month.down).cmp(&(a.month.up + a.month.down)))
                .then(a.name.cmp(&b.name))
        });
        Ok(out)
    }

    fn cycle_view(&self, c: &CycleRow, now: i64, tz: &TimeZone) -> Result<CycleView, String> {
        let range = cycle::current_at(&c.reset, now, tz)?;
        let bytes = self.kind_sums(DimKind::Subscription, range, None, None)?.get(&c.subscription).copied().unwrap_or_default();
        let used = c.mode.used(bytes);
        Ok(CycleView { config: c.clone(), start: range.start, end: range.end, used, bytes, projected: cycle::projected(used, range, now) })
    }

    pub fn save_cycle(&self, mut c: CycleRow) -> Result<i64, String> {
        c.subscription = c.subscription.trim().to_string();
        if c.subscription.is_empty() || c.subscription == split::DIRECT {
            return Err("请选择一个订阅。".into());
        }
        c.reset.check()?;
        c.alerts.retain(|a| (1..=200).contains(a));
        c.alerts.sort_unstable();
        c.alerts.dedup();
        if c.quota == Some(0) {
            c.quota = None;
        }
        lock(&self.store).save_cycle(&c)
    }

    pub fn delete_cycle(&self, id: i64) -> Result<(), String> {
        lock(&self.store).delete_cycle(id)
    }

    /// Notifies each threshold a cycle newly reached, once per cycle.
    pub fn check_alerts(&self, now: i64) -> Result<(), String> {
        let tz = TimeZone::system();
        let cycles = lock(&self.store).cycles()?;
        for c in cycles.iter().filter(|c| c.quota.is_some_and(|q| q > 0) && !c.alerts.is_empty()) {
            let (Some(id), Some(quota)) = (c.id, c.quota) else { continue };
            let v = self.cycle_view(c, now, &tz)?;
            let key = format!("alerted:{id}:{}", v.start);
            let already = lock(&self.store).meta(&key)?.and_then(|s| s.parse().ok()).unwrap_or(0);
            if let Some(threshold) = cycle::crossed(v.used, quota, &c.alerts, already) {
                lock(&self.store).set_meta(&key, &threshold.to_string())?;
                (self.sink)(Event::Alert(Alert { subscription: c.subscription.clone(), threshold, used: v.used, quota, resets_at: v.end }));
            }
        }
        Ok(())
    }

    /// Hosts over the local days `from` through `to`, of one process or of all, biggest first;
    /// past `limit` the rest is folded into the empty key. `merge` counts subdomains under their
    /// registrable domain.
    #[allow(clippy::too_many_arguments)]
    pub fn hosts(
        &self,
        from: Date,
        to: Date,
        source: Option<i64>,
        route: Option<Route>,
        process: Option<&str>,
        in_group: Option<&str>,
        view: HostView,
        limit: usize,
    ) -> Result<Vec<RankRow>, String> {
        let (from, to) = (from.to_string(), to.to_string());
        let mut rows: Vec<(String, String, Bytes)> =
            lock(&self.store).hosts(&from, &to, source, route)?.into_iter().map(|h| (h.process, h.host, h.bytes)).collect();
        for ((s, day, k), b) in lock(&self.state).pending_hosts.iter() {
            if source.is_none_or(|x| x == *s) && in_route(&k.route, route) && *day >= from && *day <= to {
                rows.push((k.process.clone(), k.host.clone(), *b));
            }
        }
        let matcher = if view == HostView::Group || in_group.is_some() { Some(self.matcher()?) } else { None };
        let mut map: HashMap<(String, bool), Bytes> = HashMap::new();
        for (p, h, b) in rows {
            if process.is_some_and(|want| want != p) {
                continue;
            }
            let group = matcher.as_ref().and_then(|m| m.group_of(&h));
            if in_group.is_some() && group != in_group {
                continue;
            }
            let key = match (view, group) {
                (HostView::Group, Some(g)) if in_group.is_none() => (g.to_string(), true),
                (HostView::Host, _) => (h, false),
                _ => (split::registrable(&h).to_string(), false),
            };
            *map.entry(key).or_default() += b;
        }
        let mut all = ranked_marked(map);
        if all.len() > limit {
            let rest = all.split_off(limit);
            let folded = rest.iter().fold(Bytes::ZERO, |a, r| a + Bytes::new(r.up, r.down));
            match all.iter_mut().find(|r| r.key.is_empty() && !r.group) {
                Some(r) => {
                    r.up += folded.up;
                    r.down += folded.down;
                }
                None => all.push(RankRow {
                    key: String::new(),
                    up: folded.up,
                    down: folded.down,
                    group: false,
                    uncollected: false,
                    unrouted: false,
                }),
            }
        }
        Ok(all)
    }

    fn attributor(&self) -> Result<Arc<Attributor>, String> {
        // Held while loading, so a change saved meanwhile clears what we load rather than being
        // overwritten by it. Saving takes the two locks one after the other, never together.
        let mut cached = lock(&self.attributor);
        if let Some(a) = cached.as_ref() {
            return Ok(a.clone());
        }
        let a = Arc::new(Attributor::new(lock(&self.store).attribution_rules()?));
        *cached = Some(a.clone());
        Ok(a)
    }

    pub fn attribution_rules(&self) -> Result<Vec<AttributionRule>, String> {
        lock(&self.store).attribution_rules()
    }

    /// Adds or changes a rule. It names traffic read from now on; what was recorded stays.
    pub fn save_attribution_rule(&self, rule: AttributionRule) -> Result<i64, String> {
        let rule = rule.checked()?;
        let result = lock(&self.store).save_attribution_rule(&rule);
        *lock(&self.attributor) = None;
        result
    }

    pub fn delete_attribution_rule(&self, id: i64) -> Result<(), String> {
        let result = lock(&self.store).delete_attribution_rule(id);
        *lock(&self.attributor) = None;
        result
    }

    fn matcher(&self) -> Result<Arc<Matcher>, String> {
        if let Some(m) = lock(&self.matcher).as_ref() {
            return Ok(m.clone());
        }
        let groups: Vec<(String, Vec<Rule>)> =
            lock(&self.store).host_groups()?.into_iter().map(|g| (g.name, g.rules.into_iter().map(|r| r.rule).collect())).collect();
        let m = Arc::new(Matcher::new(&groups));
        *lock(&self.matcher) = Some(m.clone());
        Ok(m)
    }

    /// Runs a change to the host groups and drops the compiled matcher.
    fn change_groups<T>(&self, f: impl FnOnce(&mut Store) -> Result<T, String>) -> Result<T, String> {
        let result = f(&mut lock(&self.store));
        *lock(&self.matcher) = None;
        result
    }

    pub fn host_groups(&self) -> Result<Vec<HostGroupRow>, String> {
        lock(&self.store).host_groups()
    }

    pub fn create_group(&self, name: &str, note: &str) -> Result<i64, String> {
        let name = group_name(name)?;
        self.change_groups(|s| s.create_group(&name, note.trim(), paths::now_secs()))
    }

    pub fn update_group(&self, id: i64, name: &str, note: &str) -> Result<(), String> {
        let name = group_name(name)?;
        self.change_groups(|s| s.update_group(id, &name, note.trim()))
    }

    pub fn delete_group(&self, id: i64) -> Result<(), String> {
        self.change_groups(|s| s.delete_group(id))
    }

    /// Adds entries typed or pasted into a group, one per line, read as in a text import.
    pub fn add_rules(&self, group: i64, text: &str) -> Result<AddResult, String> {
        let parsed = import::parse(text, Format::Text);
        let rules: Vec<Rule> = parsed.entries.into_iter().map(|e| e.rule).collect();
        let added = self.change_groups(|s| s.add_rules(group, &rules))?;
        Ok(AddResult { added, problems: parsed.problems })
    }

    pub fn update_rule(&self, id: i64, kind: RuleKind, value: &str) -> Result<(), String> {
        let rule = Rule::new(kind, value)?;
        self.change_groups(|s| s.update_rule(id, &rule))
    }

    pub fn delete_rules(&self, ids: &[i64]) -> Result<(), String> {
        self.change_groups(|s| s.delete_rules(ids))
    }

    pub fn import_preview(&self, text: &str, format: Format) -> Parsed {
        import::parse(text, format)
    }

    /// Adds previewed entries; those naming no group go to `default_group`.
    pub fn import_apply(&self, entries: Vec<Entry>, default_group: &str) -> Result<Added, String> {
        let fallback = if entries.iter().any(|e| e.group.is_none()) { Some(group_name(default_group)?) } else { None };
        let mut pairs = Vec::with_capacity(entries.len());
        for e in entries {
            let name = match e.group {
                Some(g) => group_name(&g)?,
                None => fallback.clone().unwrap_or_default(),
            };
            // Entries from the page are checked again rather than trusted.
            pairs.push((name, Rule::new(e.rule.kind, &e.rule.value)?));
        }
        self.change_groups(|s| s.import_rules(&pairs, paths::now_secs()))
    }

    /// Every group as JSON that `import` reads back.
    pub fn export_groups(&self) -> Result<String, String> {
        let groups: Vec<serde_json::Value> = lock(&self.store)
            .host_groups()?
            .into_iter()
            .map(|g| serde_json::json!({"name": g.name, "note": g.note, "rules": g.rules.into_iter().map(|r| r.rule).collect::<Vec<_>>()}))
            .collect();
        serde_json::to_string_pretty(&serde_json::json!({ "groups": groups })).map_err(|e| e.to_string())
    }

    /// Today's traffic of every source, for the tray.
    pub fn today(&self) -> Bytes {
        self.overview(None, None, false).map(|o| o.today).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One reading of a core's counters (and a rate for the live chart), as the collector makes.
    struct Reading {
        totals: Bytes,
        rate: Bytes,
        at: i64,
    }

    trait Record {
        fn record(&self, key: &str, id: i64, core: Option<&CoreId>, prev: Option<Bytes>, r: Reading) -> Option<LivePoint>;
    }

    impl Record for Monitor {
        fn record(&self, key: &str, id: i64, core: Option<&CoreId>, prev: Option<Bytes>, r: Reading) -> Option<LivePoint> {
            if !self.account(key, id, core, prev, r.totals, &[], r.at) {
                return None;
            }
            self.live_point(key, id, r.rate, r.at)
        }
    }

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
        let before = m.overview_at(&noon(), Some(id), None, false).unwrap().today;
        m.flush().unwrap();
        let after = m.overview_at(&noon(), Some(id), None, false).unwrap().today;

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
        let all = m.overview_at(&noon(), None, None, false).unwrap();
        let only_a = m.overview_at(&noon(), Some(a), None, false).unwrap();

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
        let days = m.history_in(&tz, Period::Day, from, to, Some(id), None, false).unwrap();
        let hours = m.history_in(&tz, Period::Hour, to, to, Some(id), None, false).unwrap();
        let weeks = m.history_in(&tz, Period::Week, from, to, Some(id), None, false).unwrap();

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
        let result = m.history_in(&tz, Period::Hour, jiff::civil::date(2026, 1, 1), jiff::civil::date(2026, 6, 1), None, None, false);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn traffic_is_split_by_where_it_went() {
        // Arrange: a baseline reading, then 1000 bytes down of which the connections account
        // for 900 (a closed connection took the rest).
        let (m, id) = monitor_with_source();
        let at = noon().timestamp().as_second();
        let dims = |outbound: &str, sub: &str, process: &str, host: &str| Dims {
            outbound: outbound.into(),
            route: split::route_of(outbound, &OutboundTypes::new()).key().into(),
            subscription: sub.into(),
            process: process.into(),
            inbound: "DEFAULT-MIXED".into(),
            host: host.into(),
        };
        m.account("a", id, None, None, Bytes::ZERO, &[], at);
        let conns = [
            (dims("HK 01", "白月光", "Chrome", "www.youtube.com"), Bytes::new(0, 600)),
            (dims("HK 01", "白月光", "Chrome", "i.ytimg.com"), Bytes::new(0, 100)),
            (dims("DIRECT", split::DIRECT, "Mail", "imap.qq.com"), Bytes::new(0, 200)),
        ];
        m.account("a", id, None, Some(Bytes::ZERO), Bytes::new(0, 1_000), &conns, at + 2);
        // Rankings and hosts are by the system's local days.
        let today = jiff::Timestamp::from_second(at).unwrap().to_zoned_datetime(TimeZone::system()).date();
        let rank = |kind| m.ranking(kind, today, today, Some(id), None).unwrap().into_iter().map(|r| (r.key, r.down)).collect::<Vec<_>>();

        // Act
        let (outbound, subscription, process) = (rank(DimKind::Outbound), rank(DimKind::Subscription), rank(DimKind::Process));
        let route: Vec<(String, u64)> = m.routes(today, today, Some(id)).unwrap().into_iter().map(|r| (r.key, r.down)).collect();
        let proxied_process: Vec<(String, u64)> =
            m.ranking(DimKind::Process, today, today, Some(id), Some(Route::Proxy)).unwrap().into_iter().map(|r| (r.key, r.down)).collect();
        let direct_hosts = m.hosts(today, today, Some(id), Some(Route::Direct), None, None, HostView::Host, 10).unwrap();
        let today_of = |r| m.overview_at(&noon(), Some(id), r, false).unwrap().today.down;
        let (all, proxied, direct) = (today_of(None), today_of(Some(Route::Proxy)), today_of(Some(Route::Direct)));
        m.flush().unwrap();
        let after = rank(DimKind::Outbound);
        let chrome = m.hosts(today, today, Some(id), None, Some("Chrome"), None, HostView::Host, 10).unwrap();
        let merged = m.hosts(today, today, Some(id), None, None, None, HostView::Domain, 1).unwrap();
        let yt = m.create_group("YouTube", "").unwrap();
        m.add_rules(yt, "+.youtube.com\nytimg.com").unwrap();
        let grouped = m.hosts(today, today, Some(id), None, None, None, HostView::Group, 10).unwrap();
        let members = m.hosts(today, today, Some(id), None, None, Some("YouTube"), HostView::Host, 10).unwrap();

        // Assert
        assert_eq!(outbound, vec![("HK 01".into(), 700), ("DIRECT".into(), 200), ("".into(), 100)]);
        assert_eq!(subscription, vec![("白月光".into(), 700), (split::DIRECT.into(), 200), ("".into(), 100)]);
        assert_eq!(process, vec![("Chrome".into(), 700), ("Mail".into(), 200), ("".into(), 100)]);
        assert_eq!(route, vec![("proxy".into(), 700), ("direct".into(), 200), ("".into(), 100)]);
        assert_eq!(proxied_process, vec![("Chrome".into(), 700)]);
        assert_eq!(direct_hosts.iter().map(|r| (r.key.as_str(), r.down)).collect::<Vec<_>>(), [("imap.qq.com", 200)]);
        assert_eq!((all, proxied, direct), (1_000, 700, 200));
        assert_eq!(after, outbound);
        assert_eq!(chrome.iter().map(|r| (r.key.as_str(), r.down)).collect::<Vec<_>>(), [("www.youtube.com", 600), ("i.ytimg.com", 100)]);
        assert_eq!(merged.iter().map(|r| (r.key.as_str(), r.down)).collect::<Vec<_>>(), [("youtube.com", 600), ("", 300)]);
        assert_eq!(
            grouped.iter().map(|r| (r.key.as_str(), r.down, r.group)).collect::<Vec<_>>(),
            [("YouTube", 700, true), ("qq.com", 200, false)]
        );
        assert_eq!(members.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["www.youtube.com", "i.ytimg.com"]);
    }

    #[test]
    fn rows_without_a_route_are_listed_apart_when_one_route_is_shown() {
        // Arrange: proxied traffic whose process rows were written before routes were kept,
        // and newer proxied traffic of curl.
        let (m, id) = monitor_with_source();
        let at = noon().timestamp().as_second();
        let row = |kind, key: &str, route: &str, down| BucketDelta {
            source: id,
            bucket: bucket_of(at),
            kind,
            key: key.into(),
            route: route.into(),
            bytes: Bytes::new(0, down),
        };
        lock(&m.store)
            .commit(
                &[
                    row(DimKind::Outbound, "HK 01", "proxy", 500),
                    row(DimKind::Process, "Chrome", "", 300),
                    row(DimKind::Process, "curl", "proxy", 200),
                ],
                &[],
                &[],
            )
            .unwrap();
        let today = jiff::Timestamp::from_second(at).unwrap().to_zoned_datetime(TimeZone::system()).date();

        // Act
        let proxied = m.ranking(DimKind::Process, today, today, Some(id), Some(Route::Proxy)).unwrap();
        let all = m.ranking(DimKind::Process, today, today, Some(id), None).unwrap();

        // Assert
        assert_eq!(
            proxied.iter().map(|r| (r.key.as_str(), r.down, r.unrouted)).collect::<Vec<_>>(),
            [("", 300, true), ("curl", 200, false)]
        );
        assert_eq!(all.iter().map(|r| (r.key.as_str(), r.down)).collect::<Vec<_>>(), [("Chrome", 300), ("curl", 200)]);
    }

    #[test]
    fn attribution_rules_name_later_traffic_without_changing_the_total() {
        // Arrange: two readings of 500 bytes down each. A container's connection through its own
        // inbound has no process; curl, through the same inbound, has one; 100 bytes of each
        // reading went through connections that closed unseen. The rule arrives in between.
        use crate::domain::attribution::Field;
        use crate::domain::split::Conn;
        let (m, id) = monitor_with_source();
        let at = noon().timestamp().as_second();
        let conn = |process: &str, host: &str, down: u64| Conn {
            id: format!("{process}{host}"),
            bytes: Bytes::new(0, down),
            chains: vec!["HK 01".into()],
            provider_chains: vec![String::new()],
            process: process.into(),
            inbound: "US-Workstation-Port".into(),
            host: host.into(),
        };
        let read = |m: &Monitor, prev: u64, total: u64| {
            let rules = m.attributor().unwrap();
            let conns: Vec<(Dims, Bytes)> = [conn("", "api.example.com", 300), conn("curl", "example.org", 100)]
                .iter()
                .map(|c| (split::dims_of(c, None, &rules, &OutboundTypes::new()), c.bytes))
                .collect();
            m.account("a", id, None, Some(Bytes::new(0, prev)), Bytes::new(0, total), &conns, at + total as i64);
        };
        m.account("a", id, None, None, Bytes::ZERO, &[], at);
        read(&m, 0, 500);
        m.save_attribution_rule(AttributionRule {
            id: None,
            name: "Container: us-workstation".into(),
            field: Field::Inbound,
            value: "US-Workstation-Port".into(),
            priority: 10,
            enabled: true,
        })
        .unwrap();
        read(&m, 500, 1_000);
        let today = jiff::Timestamp::from_second(at).unwrap().to_zoned_datetime(TimeZone::system()).date();
        let rank = |kind| m.ranking(kind, today, today, Some(id), None).unwrap();

        // Act
        let process = rank(DimKind::Process);
        let outbound = rank(DimKind::Outbound);
        let named = m.hosts(today, today, Some(id), None, Some("Container: us-workstation"), None, HostView::Host, 10).unwrap();
        let unnamed = m.hosts(today, today, Some(id), None, Some(""), None, HostView::Host, 10).unwrap();

        // Assert
        assert_eq!(
            process.iter().map(|r| (r.key.as_str(), r.down, r.uncollected)).collect::<Vec<_>>(),
            [("", 300, false), ("Container: us-workstation", 300, false), ("", 200, true), ("curl", 200, false)]
        );
        let sum = |rows: &[RankRow]| rows.iter().map(|r| r.down).sum::<u64>();
        assert_eq!(sum(&process), 1_000);
        assert_eq!(sum(&outbound), 1_000);
        assert_eq!(named.iter().map(|r| (r.key.as_str(), r.down)).collect::<Vec<_>>(), [("api.example.com", 300)]);
        assert_eq!(unnamed.iter().map(|r| (r.key.as_str(), r.down)).collect::<Vec<_>>(), [("api.example.com", 300)]);
    }

    #[test]
    fn a_cycle_counts_its_subscription_and_alerts_once_per_threshold() {
        // Arrange: a cycle with a 1000-byte quota and 600 bytes used through it so far.
        let alerts = Arc::new(Mutex::new(Vec::new()));
        let seen = alerts.clone();
        let store = Store::open_in_memory();
        let id = store.ensure_source("a", "A", 0).unwrap();
        let m = Monitor::new(store, move |e| {
            if let Event::Alert(a) = e {
                seen.lock().unwrap().push((a.subscription, a.threshold, a.used));
            }
        });
        let now = noon().timestamp().as_second();
        let dims = |sub: &str| Dims {
            outbound: "HK".into(),
            route: Route::Proxy.key().into(),
            subscription: sub.into(),
            process: String::new(),
            inbound: String::new(),
            host: "x.com".into(),
        };
        m.account("a", id, None, None, Bytes::ZERO, &[], now - 60);
        m.account(
            "a",
            id,
            None,
            Some(Bytes::ZERO),
            Bytes::new(100, 600),
            &[(dims("白月光"), Bytes::new(100, 500)), (dims("其他"), Bytes::new(0, 100))],
            now - 30,
        );
        let cycle = CycleRow {
            id: None,
            subscription: "白月光".into(),
            reset: cycle::Reset::EveryDays { start: "2026-01-01".into(), hour: 0, minute: 0, days: 365 },
            quota: Some(1_000),
            mode: cycle::CountMode::Both,
            alerts: vec![100, 50, 80, 50],
        };

        // Act
        m.save_cycle(cycle).unwrap();
        let subs = m.subscriptions(now).unwrap();
        m.check_alerts(now).unwrap();
        m.check_alerts(now).unwrap();

        // Assert
        let c = subs[0].cycle.as_ref().unwrap();
        assert_eq!((subs[0].name.as_str(), c.used, c.config.alerts.clone()), ("白月光", 600, vec![50, 80, 100]));
        assert_eq!(subs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["白月光", "其他"]);
        assert_eq!(*alerts.lock().unwrap(), vec![("白月光".to_string(), 50, 600)]);
    }

    #[test]
    fn direct_traffic_cannot_have_a_cycle() {
        // Arrange
        let (m, _) = monitor_with_source();
        let c = CycleRow {
            id: None,
            subscription: split::DIRECT.into(),
            reset: cycle::Reset::Monthly { day: 1, hour: 0, minute: 0 },
            quota: None,
            mode: cycle::CountMode::Both,
            alerts: vec![],
        };

        // Act
        let result = m.save_cycle(c);

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
        assert_eq!(again.overview_at(&noon(), Some(id), None, false).unwrap().today, Bytes::new(30, 300));
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
