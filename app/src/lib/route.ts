import { useState } from "react";
import type { Route } from "@/lib/api";

/** Which traffic the figures and charts show: all of it, or one route's. */
export type RouteFilter = "all" | Route;

export const ROUTE_FILTERS: readonly (readonly [RouteFilter, string])[] = [
  ["all", "全部"],
  ["proxy", "经代理"],
  ["direct", "直连"],
];

/**
 * Where traffic went, as recorded: through a proxy, directly, slipped between two readings (no
 * connection read carried it), or backfill (it came in while nothing was reading).
 */
export type RouteKey = Route | "" | "gap";

export const ROUTE_LABEL: Record<RouteKey, string> = { proxy: "经代理", direct: "直连", "": "漏采", gap: "补记" };

/** What the last two mean, for tooltips. */
export const ROUTE_NOTE =
  "漏采：读取期间，在两次读取之间开了又关的连接，看不出去向。补记：Mihomo-Monitor 没在读取时（未运行、断开或开始按连接统计之前）内核走过的流量，恢复读取时从计数补回，没有任何明细。";

/** Fill of each route in the breakdown bars; the empty route is traffic whose route is unknown. */
export const ROUTE_FILL: Record<RouteKey, string> = {
  proxy: "bg-route-proxy",
  direct: "bg-route-direct",
  "": "bg-route-none",
  gap: "bg-route-gap",
};

const KEY = "mihomo-monitor.route";

function read(): RouteFilter {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "proxy" || v === "direct") return v;
  } catch {
    /* storage unavailable */
  }
  return "all";
}

/** The filter, kept between pages and launches. */
export function useRouteFilter(): [RouteFilter, (f: RouteFilter) => void] {
  const [filter, setFilter] = useState<RouteFilter>(read);
  const set = (f: RouteFilter) => {
    setFilter(f);
    try {
      localStorage.setItem(KEY, f);
    } catch {
      /* the choice lasts only for this page */
    }
  };
  return [filter, set];
}

/** The route the API is asked for: `null` is all traffic. */
export function routeOf(f: RouteFilter): Route | null {
  return f === "all" ? null : f;
}
