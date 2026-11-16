import { listen } from "@tauri-apps/api/event";
import { Cpu } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { LiveChart, SERIES, StackedBars, type BarItem, type RatePoint } from "@/components/charts";
import { Page } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { api, errorText, type Bytes, type LivePoint, type Overview, type Source } from "@/lib/api";
import { bytes, bytesParts, cn, rate } from "@/lib/utils";

/** Seconds shown in the live chart. */
const LIVE_WINDOW = 300;

function Tile({ label, value }: { label: string; value: Bytes | undefined }) {
  const [num, unit] = value ? bytesParts(value.up + value.down) : ["—", ""];
  return (
    <div className="rounded-xl bg-card px-4 py-3 shadow-card">
      <div className="text-[12px] text-muted">{label}</div>
      <div className="mt-1 flex items-baseline gap-1">
        <span className="text-[22px] font-semibold tracking-[-0.01em]">{num}</span>
        <span className="text-[12px] text-muted">{unit}</span>
      </div>
      <div className="mt-1.5 space-y-0.5 text-[11.5px] text-fg-2">
        {SERIES.map((s) => (
          <div key={s.key} className="flex items-center gap-1.5">
            <span className="h-[2px] w-2.5 rounded-full" style={{ background: s.color }} />
            <span className="text-muted">{s.label}</span>
            <span className="ml-auto tabular-nums">{value ? bytes(value[s.key]) : "—"}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function Card({ title, extra, children }: { title: string; extra?: React.ReactNode; children: React.ReactNode }) {
  return (
    <section className="rounded-xl bg-card px-4 pb-3 pt-3 shadow-card">
      <div className="mb-2 flex items-center justify-between gap-3">
        <h2 className="text-[13px] font-semibold">{title}</h2>
        {extra}
      </div>
      {children}
    </section>
  );
}

function Segmented<T extends string | number | null>({ value, options, onChange }: { value: T; options: { value: T; label: string }[]; onChange: (v: T) => void }) {
  return (
    <div className="inline-flex rounded-lg bg-pill p-0.5">
      {options.map((o) => (
        <button
          key={String(o.value)}
          onClick={() => onChange(o.value)}
          className={cn("h-6 rounded-md px-2.5 text-[12px] active:!scale-100", value === o.value ? "bg-card font-medium text-fg shadow-edge dark:bg-white/[.1]" : "text-muted hover:text-fg")}
        >
          {o.label}
        </button>
      ))}
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
  for (let t = now - LIVE_WINDOW + 1; t <= now; t++) {
    const v = bySecond.get(t);
    out.push({ at: t, up: v ? v.up : null, down: v ? v.down : null });
  }
  return out;
}

export function OverviewPage({ sources, onOpenSources }: { sources: Source[] | null; onOpenSources: () => void }) {
  const [source, setSource] = useState<number | null>(null);
  const [overview, setOverview] = useState<Overview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [live, setLive] = useState<LivePoint[]>([]);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const [range, setRange] = useState<"hours" | "days">("hours");

  const recorded = (sources ?? []).filter((s) => s.id !== null);
  const online = (sources ?? []).filter((s) => s.status === "online");
  // A source that went away is no longer offered; fall back to all.
  useEffect(() => {
    if (source !== null && sources && !sources.some((s) => s.id === source)) setSource(null);
  }, [sources, source]);

  const load = useCallback(async () => {
    try {
      setOverview(await api.overview(source));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, [source]);

  useEffect(() => {
    void load();
    // Totals include traffic still waiting in memory, so a short interval keeps the tiles in
    // step with the live rate; the query is cheap and runs only while the window exists.
    const timer = window.setInterval(load, 10_000);
    window.addEventListener("focus", load);
    const un = listen("sources-changed", () => void load());
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", load);
      void un.then((f) => f());
    };
  }, [load]);

  useEffect(() => {
    let active = true;
    void api.live(source).then((pts) => active && setLive(pts));
    const un = listen<LivePoint>("traffic-sample", (e) => {
      if (source !== null && e.payload.source !== source) return;
      setLive((prev) => {
        const cutoff = e.payload.at - LIVE_WINDOW;
        return [...prev.filter((p) => p.at > cutoff), e.payload];
      });
    });
    const tick = window.setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => {
      active = false;
      window.clearInterval(tick);
      void un.then((f) => f());
    };
  }, [source]);

  const series = useMemo(() => toSeries(live, now), [live, now]);
  const latest = [...series].reverse().find((p) => p.down !== null);

  const bars: BarItem[] = useMemo(() => {
    if (!overview) return [];
    if (range === "hours") {
      return overview.hours.map((h) => {
        const d = new Date(h.start * 1000);
        const hour = d.getHours();
        return { key: String(h.start), label: hour % 3 === 0 ? String(hour) : "", title: `${hour}:00 – ${hour + 1}:00`, up: h.up, down: h.down };
      });
    }
    return overview.days.map((d, i) => {
      const [, m, day] = d.date.split("-").map(Number);
      const label = i % 5 === 4 || i === overview.days.length - 1 ? `${m}/${day}` : "";
      return { key: d.date, label, title: d.date, up: d.up, down: d.down };
    });
  }, [overview, range]);

  if (sources && recorded.length === 0 && online.length === 0) {
    return (
      <Page>
        <div className="flex flex-col items-center px-8 pt-20 text-center">
          <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-card text-muted shadow-card">
            <Cpu className="h-6 w-6" />
          </span>
          <div className="mt-4 text-[15px] font-semibold">还没有找到 mihomo 内核</div>
          <p className="mt-1.5 max-w-sm text-[12.5px] leading-5 text-muted">
            打开 Clash Verge Rev 等使用 mihomo 内核的客户端后，这里会自动开始统计。路由器上的内核或没有被自动发现的内核，可以手动添加控制地址。
          </p>
          <Button className="mt-4" variant="outline" onClick={onOpenSources}>
            查看内核
          </Button>
        </div>
      </Page>
    );
  }

  return (
    <Page>
      <div className="space-y-3 px-5 pt-1">
        {recorded.length > 1 && (
          <div className="flex items-center gap-2">
            <Segmented value={source} onChange={setSource} options={[{ value: null, label: "全部" }, ...recorded.map((s) => ({ value: s.id, label: s.name }))]} />
            {source === null && <span className="text-[11.5px] text-muted">同一流量先后经过两个内核时，合计会重复计算</span>}
          </div>
        )}
        {error && <p className="text-[12px] text-danger">{error}</p>}

        <div className="grid grid-cols-4 gap-3">
          <Tile label="今天" value={overview?.today} />
          <Tile label="本周" value={overview?.week} />
          <Tile label="本月" value={overview?.month} />
          <Tile label="今年" value={overview?.year} />
        </div>

        <Card
          title="实时速率"
          extra={
            <div className="flex items-center gap-4 text-[12px]">
              {SERIES.map((s) => (
                <span key={s.key} className="inline-flex items-center gap-1.5">
                  <span className="h-[2px] w-3 rounded-full" style={{ background: s.color }} />
                  <span className="text-muted">{s.label}</span>
                  <span className="min-w-[68px] font-medium tabular-nums">{latest ? rate(latest[s.key] ?? 0) : "—"}</span>
                </span>
              ))}
            </div>
          }
        >
          {online.length ? (
            <LiveChart points={series} />
          ) : (
            <div className="flex h-[150px] items-center justify-center text-[12.5px] text-muted">内核未连接，实时速率暂不可用</div>
          )}
        </Card>

        <Card
          title="流量"
          extra={
            <Segmented
              value={range}
              onChange={setRange}
              options={[
                { value: "hours", label: "今天" },
                { value: "days", label: "近 30 天" },
              ]}
            />
          }
        >
          <StackedBars items={bars} ariaLabel={range === "hours" ? "今天每小时的下载与上传流量" : "近 30 天每天的下载与上传流量"} />
        </Card>
      </div>
    </Page>
  );
}
