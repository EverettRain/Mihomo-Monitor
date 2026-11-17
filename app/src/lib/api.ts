import { invoke } from "@tauri-apps/api/core";

export interface Bytes {
  up: number;
  down: number;
}

export type SourceStatus = "connecting" | "online" | "offline";

/** A mihomo core, found running or recorded before. */
export interface Source {
  /** Database id; null for a core that was found but never read. */
  id: number | null;
  key: string;
  name: string;
  endpoint: string | null;
  status: SourceStatus;
  message: string | null;
  version: string | null;
  manual: boolean;
}

export interface LivePoint {
  source: number;
  /** Unix seconds. */
  at: number;
  up: number;
  down: number;
}

export interface DayPoint extends Bytes {
  /** YYYY-MM-DD, local. */
  date: string;
}

export interface HourPoint extends Bytes {
  /** Unix seconds of the hour's start. */
  start: number;
}

export interface SourceTotals {
  id: number;
  today: Bytes;
  month: Bytes;
}

export interface Overview {
  today: Bytes;
  week: Bytes;
  month: Bytes;
  year: Bytes;
  allTime: Bytes;
  /** The last 53 weeks, oldest first, ending today. */
  days: DayPoint[];
  /** Today, hour by hour. */
  hours: HourPoint[];
  /** Every recorded core, whichever one the overview is for. */
  bySource: SourceTotals[];
}

export type PeriodUnit = "hour" | "day" | "week" | "month" | "year";

/** One period of a history view. */
export interface PeriodRow extends Bytes {
  /** Unix seconds. */
  start: number;
  end: number;
  label: string;
}

/** Local dates (YYYY-MM-DD) of the first and last recorded traffic. */
export interface Bounds {
  first: string;
  last: string;
}

export type DimKind = "outbound" | "subscription" | "process" | "inbound";

/** One key of a ranking. The empty key is traffic that could not be attributed, or for hosts the many small ones folded together. */
export interface RankRow extends Bytes {
  key: string;
}

/** When a billing cycle starts over (local time, quarter hours). */
export type Reset =
  | { kind: "monthly"; day: number; hour: number; minute: number }
  | { kind: "everyDays"; start: string; hour: number; minute: number; days: number };

export type CountMode = "both" | "downloadOnly";

export interface CycleConfig {
  id: number | null;
  subscription: string;
  reset: Reset;
  /** Bytes per cycle; null for none. */
  quota: number | null;
  mode: CountMode;
  /** Percent of the quota to notify at. */
  alerts: number[];
}

export interface CycleView extends CycleConfig {
  /** Unix seconds. */
  start: number;
  end: number;
  used: number;
  bytes: Bytes;
  projected: number | null;
}

/** What the provider itself reports (all devices on the subscription). */
export interface Reported {
  upload: number;
  download: number;
  /** 0 when the provider gives no allowance. */
  total: number;
  /** Unix seconds. */
  expire: number | null;
  updated: number | null;
}

export interface Subscription {
  name: string;
  cycle: CycleView | null;
  reported: Reported | null;
  /** This calendar month. */
  month: Bytes;
}

export interface ManualSource {
  name: string;
  url: string;
  secret: string;
}

export interface Settings {
  lightweight: boolean;
  autostart: boolean;
  weekStartsSunday: boolean;
  autoDiscover: boolean;
  manualSources: ManualSource[];
}

export interface AppInfo {
  version: string;
  platform: string;
  dataDir: string;
}

/** A published release and its markdown notes. */
export interface ReleaseNote {
  version: string;
  date: string;
  body: string;
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  getSettings: () => invoke<Settings>("get_settings"),
  setLightweight: (enabled: boolean) => invoke<void>("set_lightweight", { enabled }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setWeekStartsSunday: (enabled: boolean) => invoke<void>("set_week_starts_sunday", { enabled }),
  setAutoDiscover: (enabled: boolean) => invoke<void>("set_auto_discover", { enabled }),
  listSources: () => invoke<Source[]>("list_sources"),
  addManualSource: (name: string, url: string, secret: string) => invoke<void>("add_manual_source", { name, url, secret }),
  removeManualSource: (url: string) => invoke<void>("remove_manual_source", { url }),
  overview: (source: number | null) => invoke<Overview>("overview", { source }),
  live: (source: number | null) => invoke<LivePoint[]>("live", { source }),
  history: (unit: PeriodUnit, from: string, to: string, source: number | null) => invoke<PeriodRow[]>("history", { unit, from, to, source }),
  ranking: (kind: DimKind, from: string, to: string, source: number | null) => invoke<RankRow[]>("ranking", { kind, from, to, source }),
  hosts: (from: string, to: string, source: number | null, process: string | null, merge: boolean) =>
    invoke<RankRow[]>("hosts", { from, to, source, process, merge }),
  subscriptions: () => invoke<Subscription[]>("subscriptions"),
  saveCycle: (cycle: CycleConfig) => invoke<number>("save_cycle", { cycle }),
  deleteCycle: (id: number) => invoke<void>("delete_cycle", { id }),
  dataBounds: (source: number | null) => invoke<Bounds | null>("data_bounds", { source }),
  topDays: (source: number | null, limit: number) => invoke<PeriodRow[]>("top_days", { source, limit }),
  /** Opens a save dialog; false when the user cancelled it. */
  exportCsv: (name: string, content: string) => invoke<boolean>("export_csv", { name, content }),
  releaseNotes: (after: string, upto: string) => invoke<ReleaseNote[]>("release_notes", { after, upto }),
  openReleasePage: (version: string) => invoke<void>("open_release_page", { version }),
  openDataDir: () => invoke<void>("open_data_dir"),
  restartApp: () => invoke<void>("restart_app"),
  quitApp: () => invoke<void>("quit_app"),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
