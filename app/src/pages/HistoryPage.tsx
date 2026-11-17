import { ChevronLeft, ChevronRight, Download, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { StackedBars, type BarItem } from "@/components/charts";
import { Page, PageHeader } from "@/components/PageHeader";
import { RouteShare } from "@/components/RouteShare";
import { Segmented } from "@/components/Segmented";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type Bounds, type PeriodRow, type PeriodUnit, type Source } from "@/lib/api";
import { ROUTE_FILTERS, ROUTE_LABEL, routeOf, useRouteFilter } from "@/lib/route";
import { bytes, cn, isoDate, parseDate, rate } from "@/lib/utils";

type View = PeriodUnit | "top" | "custom";

const VIEWS: readonly (readonly [View, string])[] = [
  ["hour", "小时"],
  ["day", "日"],
  ["week", "周"],
  ["month", "月"],
  ["year", "年"],
  ["top", "排行"],
  ["custom", "自定义"],
];

const TOP_N = 20;
const WEEKDAY = ["日", "一", "二", "三", "四", "五", "六"];

/** The local days a view shows around `anchor`, and how it is named. */
function windowOf(view: PeriodUnit, anchor: Date, bounds: Bounds | null): { from: Date; to: Date; title: string } {
  const y = anchor.getFullYear();
  const m = anchor.getMonth();
  switch (view) {
    case "hour":
      return { from: anchor, to: anchor, title: `${y} 年 ${m + 1} 月 ${anchor.getDate()} 日 星期${WEEKDAY[anchor.getDay()]}` };
    case "day":
      return { from: new Date(y, m, 1), to: new Date(y, m + 1, 0), title: `${y} 年 ${m + 1} 月` };
    case "week":
    case "month":
      return { from: new Date(y, 0, 1), to: new Date(y, 11, 31), title: `${y} 年` };
    case "year": {
      const first = bounds ? parseDate(bounds.first).getFullYear() : y;
      return { from: new Date(Math.min(first, y), 0, 1), to: new Date(y, 11, 31), title: "全部年份" };
    }
  }
}

/** Moves the anchor one window back (-1) or forward (+1). */
function shift(view: PeriodUnit, anchor: Date, by: number): Date {
  switch (view) {
    case "hour":
      return new Date(anchor.getFullYear(), anchor.getMonth(), anchor.getDate() + by);
    case "day":
      return new Date(anchor.getFullYear(), anchor.getMonth() + by, 1);
    default:
      return new Date(anchor.getFullYear() + by, 0, 1);
  }
}

/** Axis labels: sparse enough to stay readable for the number of columns. */
function axisLabel(view: View, row: PeriodRow, i: number, n: number): string {
  const d = new Date(row.start * 1000);
  switch (view) {
    case "hour":
      return d.getHours() % 3 === 0 ? String(d.getHours()) : "";
    case "week":
      return i % 8 === 0 ? `${d.getMonth() + 1}/${d.getDate()}` : "";
    case "month":
      return `${d.getMonth() + 1}月`;
    case "year":
      return String(d.getFullYear());
    default: {
      const every = n <= 10 ? 1 : n <= 40 ? 5 : n <= 120 ? 15 : 30;
      return (n - 1 - i) % every === 0 ? `${d.getMonth() + 1}/${d.getDate()}` : "";
    }
  }
}

/** Average rate over the part of the period that has passed. */
function averageRate(row: PeriodRow, now: number): string {
  const secs = Math.min(row.end, now) - row.start;
  return secs > 0 ? rate((row.up + row.down) / secs) : "—";
}

function csv(rows: PeriodRow[], now: number): string {
  const head = "时间,下载（字节）,上传（字节）,合计（字节）,平均速率（字节/秒）";
  const body = rows.map((r) => {
    const secs = Math.min(r.end, now) - r.start;
    const avg = secs > 0 ? Math.round((r.up + r.down) / secs) : "";
    return [`"${r.label}"`, r.down, r.up, r.up + r.down, avg].join(",");
  });
  return [head, ...body].join("\r\n") + "\r\n";
}

export function HistoryPage({ sources }: { sources: Source[] | null }) {
  const toast = useToast();
  const today = useMemo(() => {
    const n = new Date();
    return new Date(n.getFullYear(), n.getMonth(), n.getDate());
  }, []);
  const [view, setView] = useState<View>("day");
  const [anchor, setAnchor] = useState<Date>(today);
  const [custom, setCustom] = useState(() => ({ from: isoDate(new Date(today.getFullYear(), today.getMonth(), today.getDate() - 29)), to: isoDate(today) }));
  const [source, setSource] = useState<number | null>(null);
  const [route, setRoute] = useRouteFilter();
  const [bounds, setBounds] = useState<Bounds | null>(null);
  const [rows, setRows] = useState<PeriodRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);

  const recorded = (sources ?? []).filter((s): s is Source & { id: number } => s.id !== null);
  const now = Math.floor(Date.now() / 1000);

  const win = view === "top" || view === "custom" ? null : windowOf(view, anchor, bounds);
  const from = win ? isoDate(win.from) : custom.from;
  const to = win ? isoDate(win.to) : custom.to;

  const load = useCallback(async () => {
    try {
      setBounds(await api.dataBounds(source));
      const r = routeOf(route);
      if (view === "top") setRows(await api.topDays(source, r, TOP_N));
      else if (view === "custom") setRows(from <= to ? await api.history("day", from, to, source, r) : []);
      else setRows(await api.history(view, from, to, source, r));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, [view, from, to, source, route]);

  useEffect(() => {
    void load();
  }, [load]);

  const total = useMemo(() => (rows ?? []).reduce((t, r) => ({ up: t.up + r.up, down: t.down + r.down }), { up: 0, down: 0 }), [rows]);
  const shownRows = rows ?? [];
  const bars: BarItem[] = useMemo(
    () =>
      view === "top"
        ? []
        : shownRows.map((r, i) => ({ key: String(r.start), label: axisLabel(view, r, i, shownRows.length), title: r.label, up: r.up, down: r.down })),
    [shownRows, view],
  );

  const firstDate = bounds ? parseDate(bounds.first) : today;
  const canBack = win !== null && view !== "year" && win.from > firstDate;
  const canForward = win !== null && view !== "year" && win.to < today;

  const exportRows = async () => {
    if (!rows?.length) return;
    setExporting(true);
    const what = VIEWS.find(([v]) => v === view)?.[1] ?? view;
    const span = view === "top" ? `前${TOP_N}天` : from === to ? from : `${from}_${to}`;
    try {
      const scope = route === "all" ? "" : `-${ROUTE_LABEL[route]}`;
      if (await api.exportFile(`mihomo-monitor-${what}${scope}-${span}.csv`, csv(rows, now), true)) toast("已导出");
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setExporting(false);
    }
  };

  return (
    <Page>
      <PageHeader
        title="历史"
        subtitle="按小时、日、周、月、年翻看任意时段的流量。"
        actions={
          <>
            <Segmented label="流量范围" value={route} onChange={setRoute} options={ROUTE_FILTERS} />
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
            <Button size="sm" variant="outline" disabled={!rows?.length || exporting} onClick={() => void exportRows()}>
              {exporting ? <Loader2 className="animate-spin" /> : <Download />}
              导出 CSV
            </Button>
          </>
        }
      />
      <div className="space-y-5 px-5">
        <section className="list-card">
          <div className="flex flex-wrap items-center justify-between gap-3 px-4 pt-3">
            <Segmented label="视图" value={view} onChange={setView} options={VIEWS} />
            {win && view !== "year" && (
              <div className="flex items-center gap-1">
                <Button variant="ghost" size="iconSm" aria-label="上一段" disabled={!canBack} onClick={() => setAnchor((a) => shift(view as PeriodUnit, a, -1))}>
                  <ChevronLeft />
                </Button>
                <span className="min-w-[132px] text-center text-[12.5px] font-medium tabular-nums">{win.title}</span>
                <Button variant="ghost" size="iconSm" aria-label="下一段" disabled={!canForward} onClick={() => setAnchor((a) => shift(view as PeriodUnit, a, 1))}>
                  <ChevronRight />
                </Button>
                {anchor.getTime() !== today.getTime() && (
                  <Button variant="ghost" size="sm" onClick={() => setAnchor(today)}>
                    回到现在
                  </Button>
                )}
              </div>
            )}
            {view === "custom" && (
              <div className="flex items-center gap-1.5 text-[12px] text-muted">
                <Input type="date" aria-label="开始日期" value={custom.from} max={custom.to} onChange={(e) => setCustom((c) => ({ ...c, from: e.target.value }))} className="h-7 w-[140px] text-[12.5px]" />
                至
                <Input type="date" aria-label="结束日期" value={custom.to} min={custom.from} onChange={(e) => setCustom((c) => ({ ...c, to: e.target.value }))} className="h-7 w-[140px] text-[12.5px]" />
              </div>
            )}
          </div>

          <div className="border-t-0 px-4 pb-4 pt-3">
            {error && <p className="selectable mb-2 text-[12px] text-danger">{error}</p>}
            <div className="mb-3 flex flex-wrap items-baseline gap-x-5 gap-y-1 text-[12px] text-muted">
              <span>
                {view === "top" ? `用量最高的 ${TOP_N} 天` : "合计"}
                {view !== "top" && <span className="ml-1.5 font-mono text-[15px] font-semibold tabular-nums text-fg">{bytes(total.up + total.down)}</span>}
              </span>
              {view !== "top" && (
                <>
                  <span>
                    下载 <span className="font-mono tabular-nums text-fg-2">{bytes(total.down)}</span>
                  </span>
                  <span>
                    上传 <span className="font-mono tabular-nums text-fg-2">{bytes(total.up)}</span>
                  </span>
                </>
              )}
              {bounds && <span className="ml-auto">记录自 {bounds.first}</span>}
            </div>
            {view !== "top" && from <= to && (
              <div className="-mt-1.5 mb-3">
                <RouteShare from={from} to={to} source={source} filter={route} version={rows} />
              </div>
            )}
            {view !== "top" && <StackedBars items={bars} tableToggle={false} ariaLabel={`${win?.title ?? `${from} 至 ${to}`}的下载与上传流量`} />}
            <PeriodTable rows={shownRows} ranked={view === "top"} now={now} loading={rows === null} />
          </div>
        </section>
      </div>
    </Page>
  );
}

function PeriodTable({ rows, ranked, now, loading }: { rows: PeriodRow[]; ranked: boolean; now: number; loading: boolean }) {
  const max = Math.max(1, ...rows.map((r) => r.up + r.down));
  if (loading) return <p className="py-6 text-center text-[12px] text-muted">正在读取…</p>;
  if (!rows.length) return <p className="py-6 text-center text-[12px] text-muted">这段时间没有记录</p>;
  return (
    <div className={cn("overflow-hidden rounded-xl border border-line-2", !ranked && "mt-3")}>
      <table className="w-full text-[12px]">
        <thead className="bg-card-2 text-[11.5px] text-muted">
          <tr>
            {ranked && <th className="w-10 px-3 py-1.5 text-left font-normal">#</th>}
            <th className="px-3 py-1.5 text-left font-normal">时间</th>
            <th className="px-3 py-1.5 text-right font-normal">下载</th>
            <th className="px-3 py-1.5 text-right font-normal">上传</th>
            <th className="px-3 py-1.5 text-right font-normal">合计</th>
            <th className="w-[88px] px-3 py-1.5 font-normal">
              <span className="sr-only">占比</span>
            </th>
            <th className="hidden px-3 py-1.5 text-right font-normal min-[720px]:table-cell">平均速率</th>
          </tr>
        </thead>
        <tbody className="font-mono tabular-nums text-fg-2">
          {rows.map((r, i) => {
            const empty = r.up + r.down === 0;
            const future = r.start > now;
            return (
              <tr key={r.start} className={cn("border-t border-line-2", (empty || future) && "text-faint")}>
                {ranked && <td className="px-3 py-1 text-muted">{i + 1}</td>}
                <td className="px-3 py-1 font-sans text-fg">{r.label}</td>
                <td className="px-3 py-1 text-right">{future ? "" : bytes(r.down)}</td>
                <td className="px-3 py-1 text-right">{future ? "" : bytes(r.up)}</td>
                <td className="px-3 py-1 text-right text-fg">{future ? "" : bytes(r.up + r.down)}</td>
                <td className="px-3 py-1" aria-hidden>
                  {!future && (
                    <div className="h-1.5 overflow-hidden rounded-full bg-pill">
                      {!empty && <div className="h-full rounded-full bg-accent" style={{ width: `${Math.max(((r.up + r.down) / max) * 100, 2)}%` }} />}
                    </div>
                  )}
                </td>
                <td className="hidden px-3 py-1 text-right min-[720px]:table-cell">{future || empty ? "" : averageRate(r, now)}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
