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
