import type { CycleView, Reset } from "@/lib/api";

export const GB = 1024 ** 3;

const pad = (n: number) => String(n).padStart(2, "0");

/** `11/15 08:00`, local. */
export function shortTime(sec: number): string {
  const d = new Date(sec * 1000);
  return `${d.getMonth() + 1}/${d.getDate()} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** `2027-01-01`, local. */
export function dateOf(sec: number): string {
  const d = new Date(sec * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Time left until `end`: `12 天 9 小时`, `5 小时`, `不到 1 小时`. */
export function remaining(end: number, now: number): string {
  const left = end - now;
  if (left <= 0) return "已到期";
  const days = Math.floor(left / 86400);
  const hours = Math.floor((left % 86400) / 3600);
  if (days > 0) return hours > 0 ? `${days} 天 ${hours} 小时` : `${days} 天`;
  return hours > 0 ? `${hours} 小时` : "不到 1 小时";
}

/** The rule in words: `每月 15 日 00:00`, `自 2026-11-01 起每 30 天 08:00`. */
export function describeReset(r: Reset): string {
  const time = `${pad(r.hour)}:${pad(r.minute)}`;
  return r.kind === "monthly" ? `每月 ${r.day} 日 ${time}` : `自 ${r.start} 起每 ${r.days} 天 ${time}`;
}

/** Share of the quota used, 0–100+; null without a quota. */
export function percentUsed(c: CycleView): number | null {
  return c.quota ? (c.used / c.quota) * 100 : null;
}

/** Bar color by how close the cycle is to its quota. */
export function meterTone(pct: number | null): string {
  if (pct === null) return "bg-accent";
  if (pct >= 100) return "bg-danger";
  if (pct >= 80) return "bg-warn";
  return "bg-accent";
}
