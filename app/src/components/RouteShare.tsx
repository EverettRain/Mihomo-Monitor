import { useEffect, useState } from "react";
import { api, type RankRow } from "@/lib/api";
import { ROUTE_FILL, ROUTE_LABEL, ROUTE_NOTE, type RouteFilter, type RouteKey } from "@/lib/route";
import { bytes, cn } from "@/lib/utils";

const ORDER: RouteKey[] = ["proxy", "direct", "", "gap"];

/** Of what was counted while reading went on, the share connections read account for. */
function coverage(rows: RankRow[]): number | null {
  const of = (key: string) => rows.filter((r) => r.key === key).reduce((t, r) => t + r.up + r.down, 0);
  const seen = of("proxy") + of("direct");
  const read = seen + of("");
  return read > 0 ? seen / read : null;
}

/**
 * How much of the traffic counted while Mihomo-Monitor was reading the connections read account
 * for: lately, and over the days shown. Backfill is left out, so a long gap or the days before
 * connections were read do not hide how well it does now.
 */
export function Coverage({ from, to, source, version }: { from: string; to: string; source: number | null; version?: unknown }) {
  const [recent, setRecent] = useState<number | null>(null);
  const [span, setSpan] = useState<number | null>(null);
  useEffect(() => {
    let active = true;
    api.recentRoutes(source, 60).then(
      (r) => active && setRecent(coverage(r)),
      () => active && setRecent(null),
    );
    api.routes(from, to, source).then(
      (r) => active && setSpan(coverage(r)),
      () => active && setSpan(null),
    );
    return () => {
      active = false;
    };
  }, [from, to, source, version]);
  if (recent === null && span === null) return null;
  const pct = (v: number | null) => (v === null ? "—" : `${(v * 100).toFixed(v > 0.995 ? 1 : 0)}%`);
  return (
    <span
      className="inline-flex items-baseline gap-x-1.5 text-[12px] text-muted"
      title="读取期间，内核计入的流量里有多少能对应到读到的连接（节点、进程、域名等明细都来自这些连接）。未运行期间的补记不算在内；按 15 分钟记录，「近 1 小时」约为最近 60–75 分钟。缩短设置里的采样间隔可以减少漏采。"
    >
      明细覆盖率
      <span>近 1 小时</span>
      <span className="font-mono tabular-nums text-fg-2">{pct(recent)}</span>
      <span className="text-faint">·</span>
      <span>这段时间</span>
      <span className="font-mono tabular-nums text-fg-2">{pct(span)}</span>
    </span>
  );
}

/**
 * How the traffic of the local days `from` through `to` divides by route, with the filtered one
 * stressed. Shown beside any figure narrowed to one route, so what it leaves out stays in sight.
 */
export function RouteShare({
  from,
  to,
  source,
  filter,
  version,
  swatches,
}: {
  from: string;
  to: string;
  source: number | null;
  filter: RouteFilter;
  version?: unknown;
  /** Marks each route with its bar color, as the legend of bars split by route. */
  swatches?: boolean;
}) {
  const [rows, setRows] = useState<RankRow[] | null>(null);
  useEffect(() => {
    let active = true;
    api.routes(from, to, source).then(
      (r) => active && setRows(r),
      () => active && setRows(null),
    );
    return () => {
      active = false;
    };
  }, [from, to, source, version]);

  const total = (rows ?? []).reduce((t, r) => t + r.up + r.down, 0);
  if (!rows || total === 0) return null;
  const of = (key: string) => rows.filter((r) => r.key === key).reduce((t, r) => t + r.up + r.down, 0);
  return (
    <span
      className="inline-flex flex-wrap items-baseline gap-x-2 text-[12px] text-muted"
      title={ROUTE_NOTE}
    >
      {ORDER.map((key, i) => {
        const v = of(key);
        if ((key === "" || key === "gap") && v === 0) return null;
        return (
          <span key={key} className={cn(filter !== "all" && filter === key && "font-medium text-fg")}>
            {i > 0 && <span className="mr-2 text-faint">·</span>}
            {swatches && <span aria-hidden className={cn("mr-1.5 inline-block h-2 w-2 rounded-[2px] align-[1px]", ROUTE_FILL[key])} />}
            {ROUTE_LABEL[key]} <span className="font-mono tabular-nums">{bytes(v)}</span>
            <span className="ml-1 tabular-nums">{Math.round((v / total) * 100)}%</span>
          </span>
        );
      })}
    </span>
  );
}
