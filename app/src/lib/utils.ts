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

/** A per-second rate. */
export function rate(n: number): string {
  return `${bytes(n)}/s`;
}

/**
 * Splits a size into number and unit, for figures that set the unit smaller. Rounded to fewer
 * digits than `bytes`, as an axis or headline wants.
 */
export function bytesParts(n: number): [string, string] {
  const [v, u] = bytes(n).split(" ");
  return [v, u];
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
