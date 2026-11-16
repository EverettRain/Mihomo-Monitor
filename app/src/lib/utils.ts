import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Binary units, as proxy clients and most airports count: 1 GB = 1024 MB. */
export function bytes(n: number): string {
  let v = n;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  if (i === 0) return `${Math.round(v)} B`;
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(2)} ${UNITS[i]}`;
}

/** Three significant digits, for secondary figures in tight spaces: 17.4 GB, 1.13 GB, 638 GB. */
export function bytesShort(n: number): string {
  let v = n;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  if (i === 0) return `${Math.round(v)} B`;
  return `${v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2)} ${UNITS[i]}`;
}

/** A local date as YYYY-MM-DD. */
export function isoDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** Parses YYYY-MM-DD as a local date (midnight). */
export function parseDate(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

/** A per-second rate. */
export function rate(n: number): string {
  return `${bytes(n)}/s`;
}

/** A clean axis top: 1, 2 or 5 × a power of 1024-based unit steps rounded in that unit. */
export function niceBytesMax(v: number): number {
  if (v <= 0) return 1024;
  let unit = 1;
  while (v / unit >= 1024 && unit < 1024 ** 4) unit *= 1024;
  const x = v / unit;
  const p = 10 ** Math.floor(Math.log10(x));
  const m = [1, 2, 5, 10].find((k) => k * p >= x) ?? 10;
  return m * p * unit;
}
