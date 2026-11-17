import { useEffect, useState } from "react";
import { api, type RankRow, type Route } from "@/lib/api";
import { ROUTE_FILL, ROUTE_LABEL, type RouteFilter } from "@/lib/route";
import { bytes, cn } from "@/lib/utils";

const ORDER: (Route | "")[] = ["proxy", "direct", ""];

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
      title="「未区分」是两次读取之间就关闭的连接，以及开始按连接统计之前的流量，看不出是否经过代理。"
    >
      {ORDER.map((key, i) => {
        const v = of(key);
        if (key === "" && v === 0) return null;
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
