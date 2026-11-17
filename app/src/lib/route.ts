import { useState } from "react";
import type { Route } from "@/lib/api";

/** Which traffic the figures and charts show: all of it, or one route's. */
export type RouteFilter = "all" | Route;

export const ROUTE_FILTERS: readonly (readonly [RouteFilter, string])[] = [
  ["all", "全部"],
  ["proxy", "经代理"],
  ["direct", "直连"],
];

export const ROUTE_LABEL: Record<Route | "", string> = { proxy: "经代理", direct: "直连", "": "未区分" };

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
