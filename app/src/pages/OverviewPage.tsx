import { listen } from "@tauri-apps/api/event";
import { ChartColumn, Cpu } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LiveChart, SERIES, StackedBars, type BarItem, type RatePoint } from "@/components/charts";
import { Heatmap } from "@/components/Heatmap";
import { RouteShare } from "@/components/RouteShare";
import { Segmented } from "@/components/Segmented";
import { Group, Page, PageHeader } from "@/components/PageHeader";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/input";
import { api, errorText, type Bytes, type DayPoint, type LivePoint, type Overview, type Source, type Subscription } from "@/lib/api";
import { usePageActive } from "@/lib/pageActive";
import { meterTone, percentUsed, remaining } from "@/lib/cycle";
import { ROUTE_FILTERS, routeOf, useRouteFilter } from "@/lib/route";
import { bytes, bytesShort, cn, isoDate, rate } from "@/lib/utils";

/** Seconds shown in the live chart. */
const LIVE_WINDOW = 300;
/** Missing seconds bridged in the live chart. */
const MAX_GAP = 2;
const RANGES = [
  ["today", "今天"],
  [7, "7 天"],
  [30, "30 天"],
  [90, "90 天"],
] as const;
type RangeId = (typeof RANGES)[number][0];

const sum = (b: Bytes) => b.up + b.down;
/** The upload share; download is nearly all the rest, and the trend view has both in full. */
const split = (b: Bytes) => `上传 ${bytesShort(b.up)}`;

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="min-w-0 bg-card px-4 py-3">
      <div className="truncate font-mono text-[19px] font-semibold tabular-nums tracking-[-0.01em]">{value}</div>
      <div className="mt-0.5 truncate text-[11.5px] text-muted">{label}</div>
      {sub && <div className="truncate text-[10.5px] text-muted">{sub}</div>}
    </div>
  );
}

/** Live points of the chosen source(s), one per second over the window; gaps stay null. */
function toSeries(points: LivePoint[], now: number): RatePoint[] {
  const bySecond = new Map<number, { up: number; down: number }>();
  for (const p of points) {
    const v = bySecond.get(p.at) ?? { up: 0, down: 0 };
    v.up += p.up;
    v.down += p.down;
    bySecond.set(p.at, v);
  }
  const out: RatePoint[] = [];
  let last: { up: number; down: number } | null = null;
  let missing = 0;
  for (let t = now - LIVE_WINDOW + 1; t <= now; t++) {
    const v = bySecond.get(t);
    if (v) {
      last = v;
      missing = 0;
    } else {
      missing++;
    }
    // Readings arrive once a second but are stamped on arrival, so now and then a second has
    // two and the next none; bridge such a hiccup, break the line only when the core is gone.
    const shown = v ?? (last && missing <= MAX_GAP ? last : null);
    out.push({ at: t, up: shown ? shown.up : null, down: shown ? shown.down : null });
  }
  return out;
}

/** The last `n` local days, today included, as API dates. */
function days(n: number): [string, string] {
  const today = new Date();
  return [isoDate(new Date(today.getFullYear(), today.getMonth(), today.getDate() - (n - 1))), isoDate(today)];
}

function dayBars(days: DayPoint[], n: number): BarItem[] {
  const shown = days.slice(-n);
  const every = n <= 7 ? 1 : n <= 30 ? 5 : 15;
  return shown.map((d, i) => {
    const [, m, day] = d.date.split("-").map(Number);
    const fromEnd = shown.length - 1 - i;
    return { key: d.date, label: fromEnd % every === 0 ? `${m}/${day}` : "", title: d.date, up: d.up, down: d.down };
  });
}

function useLive(source: number | null, shown: boolean) {
  const [live, setLive] = useState<LivePoint[]>([]);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  // Hidden, it neither listens nor ticks; shown again, it reads the last minutes afresh.
  useEffect(() => {
    if (!shown) return;
    let active = true;
    void api.live(source).then((pts) => active && setLive(pts));
    const un = listen<LivePoint>("traffic-sample", (e) => {
      if (source !== null && e.payload.source !== source) return;
      setLive((prev) => [...prev.filter((p) => p.at > e.payload.at - LIVE_WINDOW), e.payload]);
    });
    const tick = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => {
      active = false;
      window.clearInterval(tick);
      void un.then((f) => f());
    };
  }, [source, shown]);
  return useMemo(() => toSeries(live, now), [live, now]);
}

const STATUS_TAG = { online: ["正在统计", "ok"], connecting: ["正在连接", "warn"], offline: ["未连接", undefined] } as const;

/** Each subscription's billing cycle at a glance, as Hydra shows an account's limits. */
function Cycles({ subs, onOpen }: { subs: Subscription[]; onOpen: () => void }) {
  const now = Math.floor(Date.now() / 1000);
  const withCycle = subs.filter((s) => s.cycle);
  if (!withCycle.length) return null;
  return (
    <Group label="计费周期">
      {withCycle.map((s) => {
        const c = s.cycle!;
        const pct = percentUsed(c);
        return (
          <button key={s.name} onClick={onOpen} className="block w-full px-4 py-3 text-left hover:bg-pill/50 active:!scale-100">
            <div className="flex items-baseline justify-between gap-3">
              <span className="truncate text-[13px] font-medium">{s.name}</span>
              <span className="shrink-0 font-mono text-[12px] tabular-nums">
                {bytes(c.used)}
                {c.quota ? <span className="text-muted"> / {bytes(c.quota)}</span> : null}
              </span>
            </div>
            <div className="mt-1.5 h-1 overflow-hidden rounded-full bg-pill">
              <div className={cn("h-full rounded-full", meterTone(pct))} style={{ width: `${pct === null ? 0 : Math.min(100, Math.max(pct, 1))}%` }} />
            </div>
            <div className="mt-1 text-[10.5px] text-muted">
              {[`还剩 ${remaining(c.end, now)}`, pct === null ? "" : `${pct.toFixed(0)}%`, c.projected === null ? "" : `预计周期末 ${bytes(c.projected)}`].filter(Boolean).join(" · ")}
            </div>
          </button>
        );
      })}
    </Group>
  );
}

export function OverviewPage({
  sources,
  onOpenSources,
  onOpenSubscriptions,
}: {
  sources: Source[] | null;
  onOpenSources: () => void;
  onOpenSubscriptions: () => void;
}) {
  const shown = usePageActive();
  const [source, setSource] = useState<number | null>(null);
  const [route, setRoute] = useRouteFilter();
  const [overview, setOverview] = useState<Overview | null>(null);
  const [subs, setSubs] = useState<Subscription[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"overview" | "trend">("overview");
  const [range, setRange] = useState<RangeId>("today");
  const [weekStartsSunday, setWeekStartsSunday] = useState(false);
  const panel = useRef<HTMLElement>(null);

  const recorded = (sources ?? []).filter((s): s is Source & { id: number } => s.id !== null);
  const online = (sources ?? []).filter((s) => s.status === "online");
  // A source that went away is no longer offered; fall back to all.
  useEffect(() => {
    if (source !== null && sources && !sources.some((s) => s.id === source)) setSource(null);
  }, [sources, source]);

  const load = useCallback(async () => {
    try {
      setOverview(await api.overview(source, routeOf(route)));
      setSubs(await api.subscriptions());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, [source, route]);

  useEffect(() => {
    if (!shown) return;
    void load();
    void api.getSettings().then((s) => setWeekStartsSunday(s.weekStartsSunday)).catch(() => {});
    // Totals include traffic still waiting in memory, so a short interval keeps them in step
    // with the live rate; the query is cheap and runs only while the window exists.
    const timer = window.setInterval(load, 10_000);
    window.addEventListener("focus", load);
    const un = listen("sources-changed", () => void load());
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", load);
      void un.then((f) => f());
    };
  }, [load, shown]);

  const series = useLive(source, shown);
  const latest = [...series].reverse().find((p) => p.down !== null);

  const stats = useMemo(() => {
    if (!overview) return null;
    const month = overview.days.slice(-30);
    const last30 = month.reduce((t, d) => ({ up: t.up + d.up, down: t.down + d.down }), { up: 0, down: 0 });
    const active = month.filter((d) => d.up + d.down > 0).length;
    const peak = overview.days.reduce<DayPoint | null>((best, d) => (!best || sum(d) > sum(best) ? d : best), null);
    return { last30, average: active ? sum(last30) / active : 0, peak: peak && sum(peak) > 0 ? peak : null };
  }, [overview]);

  const bars: BarItem[] = useMemo(() => {
    if (!overview) return [];
    if (range !== "today") return dayBars(overview.days, range);
    return overview.hours.map((h) => {
      const hour = new Date(h.start * 1000).getHours();
      return { key: String(h.start), label: hour % 3 === 0 ? String(hour) : "", title: `${hour}:00 – ${hour + 1}:00`, up: h.up, down: h.down };
    });
  }, [overview, range]);

  if (sources && recorded.length === 0 && online.length === 0) {
    return (
      <Page>
        <PageHeader title="概览" subtitle="mihomo 内核的流量：按本地时间汇总成小时、日、周、月、年。" />
        <div className="px-5">
          <div className="flex flex-col items-center rounded-xl border border-dashed border-line px-8 py-12 text-center">
            <span className="flex h-11 w-11 items-center justify-center rounded-xl bg-card text-muted shadow-card">
              <Cpu className="h-5 w-5" />
            </span>
            <div className="mt-3 text-[14px] font-semibold">还没有找到 mihomo 内核</div>
            <p className="mt-1 max-w-sm text-[12.5px] leading-5 text-muted">
              打开 Clash Verge Rev 等使用 mihomo 内核的客户端后，这里会自动开始统计。路由器上的内核或没有被自动发现的内核，可以手动添加控制地址。
            </p>
            <Button className="mt-4" variant="outline" size="sm" onClick={onOpenSources}>
              查看内核
            </Button>
          </div>
        </div>
      </Page>
    );
  }

  const chosen = recorded.find((s) => s.id === source);
  const subtitle = chosen
    ? `${chosen.name} 的流量，按本地时间汇总。`
    : recorded.length > 1
      ? `${recorded.length} 个内核的流量合计，按本地时间汇总。`
      : "mihomo 内核的流量：按本地时间汇总成小时、日、周、月、年。";

  return (
    <Page>
      <PageHeader title="概览" subtitle={subtitle} actions={<Segmented label="流量范围" value={route} onChange={setRoute} options={ROUTE_FILTERS} />} />
      <div className="space-y-5 px-5">
        <section ref={panel} className="list-card">
          <div className="flex items-center justify-between gap-3 px-4 pt-3">
            <Segmented
              label="视图"
              value={tab}
              onChange={setTab}
              options={[
                ["overview", "总览"],
                ["trend", "趋势"],
              ]}
            />
            <div className="flex items-center gap-1.5">
              {error && (
                <span className="text-[11px] text-danger" title={error}>
                  读取失败
                </span>
              )}
              {tab === "trend" && <Segmented label="时间范围" value={range} onChange={setRange} options={RANGES} />}
              {recorded.length > 1 && (
                <Select
                  aria-label="查看的内核"
                  value={source ?? ""}
                  onChange={(e) => setSource(e.target.value ? Number(e.target.value) : null)}
                  className="h-7 w-auto min-w-[120px] max-w-[200px] truncate text-[12.5px]"
                >
                  <option value="">全部内核</option>
                  {recorded.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name}
                    </option>
                  ))}
                </Select>
              )}
            </div>
          </div>

          {tab === "overview" ? (
            <>
              <div className="m-4 grid grid-cols-2 gap-px overflow-hidden rounded-xl border border-line-2 bg-line-2 min-[620px]:grid-cols-4">
                <Stat label="今天" value={overview ? bytes(sum(overview.today)) : "—"} sub={overview ? split(overview.today) : undefined} />
                <Stat label="本周" value={overview ? bytes(sum(overview.week)) : "—"} sub={overview ? split(overview.week) : undefined} />
                <Stat label="本月" value={overview ? bytes(sum(overview.month)) : "—"} sub={overview ? split(overview.month) : undefined} />
                <Stat label="今年" value={overview ? bytes(sum(overview.year)) : "—"} sub={overview ? split(overview.year) : undefined} />
                <Stat label="近 30 天" value={stats ? bytes(sum(stats.last30)) : "—"} sub={stats ? split(stats.last30) : undefined} />
                <Stat label="日均" value={stats ? bytes(stats.average) : "—"} sub="近 30 天有流量的日子" />
                <Stat label="峰值单日" value={stats?.peak ? bytes(sum(stats.peak)) : "—"} sub={stats?.peak?.date ?? "近一年"} />
                <Stat label="累计" value={overview ? bytes(sum(overview.allTime)) : "—"} sub={overview ? split(overview.allTime) : undefined} />
              </div>
              <div className="-mt-1 mb-4 flex flex-wrap items-baseline gap-x-2 px-4 text-[12px] text-muted">
                近 30 天
                <RouteShare from={days(30)[0]} to={days(30)[1]} source={source} filter={route} version={overview} />
              </div>

              <div className="px-4">
                <div className="mb-1 flex items-center justify-between gap-3">
                  <span className="text-[12.5px] font-medium text-fg-2">
                    实时速率
                    {route !== "all" && <span className="ml-1.5 text-[11.5px] font-normal text-muted">全部流量，实时速率无法按是否经代理区分</span>}
                  </span>
                  <div className="flex items-center gap-4 text-[11.5px]">
                    {SERIES.map((s) => (
                      <span key={s.key} className="inline-flex items-center gap-1.5">
                        <span className="h-[2px] w-3 rounded-full" style={{ background: s.color }} />
                        <span className="text-muted">{s.label}</span>
                        <span className="min-w-[72px] font-mono tabular-nums">{latest ? rate(latest[s.key] ?? 0) : "—"}</span>
                      </span>
                    ))}
                  </div>
                </div>
                {online.length ? (
                  <LiveChart points={series} />
                ) : (
                  <div className="flex h-[150px] items-center justify-center rounded-xl border border-dashed border-line text-[12px] text-muted">
                    内核未连接，实时速率暂不可用
                  </div>
                )}
              </div>

              <div className="px-4 pb-4 pt-5">
                <div className="mb-2 text-[12.5px] font-medium text-fg-2">
                  流量活动
                  {route !== "all" && <span className="ml-1.5 text-[11.5px] font-normal text-muted">开始按连接统计之前的流量分不出是否经代理，不计入</span>}
                </div>
                {overview && <Heatmap days={overview.days} weekStartsSunday={weekStartsSunday} />}
              </div>
            </>
          ) : (
            <div className="border-t-0 px-4 pb-4 pt-3">
              <StackedBars items={bars} ariaLabel={range === "today" ? "今天每小时的下载与上传流量" : `近 ${range} 天每天的下载与上传流量`} />
              <div className="mt-2">
                <RouteShare from={days(range === "today" ? 1 : range)[0]} to={days(range === "today" ? 1 : range)[1]} source={source} filter={route} version={overview} />
              </div>
            </div>
          )}
        </section>

        <Cycles subs={subs} onOpen={onOpenSubscriptions} />

        {recorded.length > 0 && (
          <Group label="内核">
            {recorded.map((s) => {
              const t = overview?.bySource.find((b) => b.id === s.id);
              const [status, tone] = STATUS_TAG[s.status];
              return (
                <div key={s.key} className="flex items-center gap-3 px-4 py-3">
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-pill text-fg-2">
                    <Cpu className="h-3.5 w-3.5" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="truncate text-[13.5px] font-medium">{s.name}</span>
                      <Tag tone={tone}>{status}</Tag>
                      {s.version && <Tag>{s.version}</Tag>}
                    </div>
                    <div className="truncate text-[11px] text-muted">
                      {[t ? `今天 ${bytes(sum(t.today))}` : "", t ? `本月 ${bytes(sum(t.month))}` : "", s.endpoint ?? ""].filter(Boolean).join(" · ")}
                    </div>
                  </div>
                  <Button
                    variant="ghost"
                    size="iconSm"
                    aria-label={`查看 ${s.name} 的趋势`}
                    title="查看趋势"
                    onClick={() => {
                      setSource(s.id);
                      setTab("trend");
                      panel.current?.scrollIntoView({ behavior: "smooth", block: "start" });
                    }}
                  >
                    <ChartColumn />
                  </Button>
                </div>
              );
            })}
          </Group>
        )}

        <p className="px-1 text-[11.5px] leading-relaxed text-muted">
          流量取自内核的累计计数，按 15 分钟记录、永久保存。第一次连接某个内核时，它此前的流量不计入；之后 Mihomo-Monitor
          退出或电脑睡眠期间的流量，只要内核没有重启，会在下次读取时补回，并记在补回的那一刻。同一份流量先后经过两个内核时，「全部内核」会重复计算。1 GB = 1024 MB。
        </p>
      </div>
    </Page>
  );
}
