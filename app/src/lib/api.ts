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

/** Through a proxy or not. Traffic no connection accounts for has no route; `null` asks for all traffic. */
export type Route = "proxy" | "direct";

/** One key of a ranking. The empty key is traffic that could not be attributed, or for hosts the many small ones folded together. */
export interface RankRow extends Bytes {
  key: string;
  /** The key is a host group's name. */
  group?: boolean;
  /**
   * Set on rows of traffic no key accounts for (key empty), and why: `unseen` slipped between two
   * readings, `backfill` came in while nothing was reading (or before connections were read at
   * all), `unrouted` is one route's traffic recorded before this dimension kept routes.
   */
  remainder?: "unseen" | "backfill" | "unrouted";
  /** Of up + down, what went through a proxy and what went directly; the rest has no known route. */
  proxy: number;
  direct: number;
}

/** What an attribution rule looks at: so far the inbound's name, compared exactly. */
export type AttributionField = "inbound";

/** Names traffic of connections the core gave no process, by a fact it does record. */
export interface AttributionRule {
  /** null for a rule not saved yet. */
  id: number | null;
  /** Shown as the process of the traffic the rule matches. */
  name: string;
  field: AttributionField;
  value: string;
  /** Rules are tried from the lowest number up. */
  priority: number;
  enabled: boolean;
}

/** How hosts are listed: as connected to, under their registrable domain, or under their group. */
export type HostView = "host" | "domain" | "group";

/** Clash's matching rules: exact domain, domain with subdomains, keyword, address or range. */
export type RuleKind = "domain" | "suffix" | "keyword" | "cidr";

export interface HostRule {
  id: number;
  kind: RuleKind;
  value: string;
}

export interface HostGroup {
  id: number;
  name: string;
  note: string;
  rules: HostRule[];
}

export interface Added {
  groups: number;
  rules: number;
  duplicates: number;
}

export interface ImportProblem {
  /** 1-based line or item; 0 when there is none. */
  at: number;
  text: string;
  reason: string;
}

export type ImportFormat = "auto" | "text" | "csv" | "json" | "yaml";

export interface ImportEntry {
  /** null goes to the group chosen for the import. */
  group: string | null;
  kind: RuleKind;
  value: string;
}

/** What importing a rules file changes in one part of it, by name. */
export interface PartPlan {
  added: string[];
  changed: string[];
  removed: string[];
  unchanged: number;
}

/** What reading a configuration file changed; a part the file leaves out is null and stays as it is. */
export interface RulesPlan {
  groups: PartPlan | null;
  attribution: PartPlan | null;
  problems: ImportProblem[];
}

/** The rule configuration in use: a YAML file holding the host groups and attribution rules. */
export interface RulesProfile {
  name: string;
  path: string;
  groups: number;
  rules: number;
  /** What reading its file again changed, when it was edited outside the app. */
  changed: RulesPlan | null;
  /** Why its file could not be read; the groups and rules stay as they were. */
  error: string | null;
}

export interface RulesProfileSummary {
  name: string;
  active: boolean;
  groups: number;
  rules: number;
  error: string | null;
}

export interface ImportPreview {
  format: ImportFormat;
  entries: ImportEntry[];
  problems: ImportProblem[];
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
  /** How often connections are read, in milliseconds. */
  sampleMs: number;
}

export interface AppInfo {
  version: string;
  platform: string;
  dataDir: string;
}

/** A published release and its markdown notes. */
/** The statistics on disk were written by a later version; this build only offers to update. */
export interface NewerData {
  /** The data's schema version. */
  found: number;
  /** The newest this build reads. */
  supported: number;
}

export interface ReleaseNote {
  version: string;
  date: string;
  body: string;
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  /** Set when the data comes from a later version; then no other command but updating may be used. */
  newerData: () => invoke<NewerData | null>("newer_data"),
  getSettings: () => invoke<Settings>("get_settings"),
  setLightweight: (enabled: boolean) => invoke<void>("set_lightweight", { enabled }),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setWeekStartsSunday: (enabled: boolean) => invoke<void>("set_week_starts_sunday", { enabled }),
  setAutoDiscover: (enabled: boolean) => invoke<void>("set_auto_discover", { enabled }),
  setSampleInterval: (ms: number) => invoke<void>("set_sample_interval", { ms }),
  listSources: () => invoke<Source[]>("list_sources"),
  addManualSource: (name: string, url: string, secret: string) => invoke<void>("add_manual_source", { name, url, secret }),
  removeManualSource: (url: string) => invoke<void>("remove_manual_source", { url }),
  overview: (source: number | null, route: Route | null) => invoke<Overview>("overview", { source, route }),
  live: (source: number | null) => invoke<LivePoint[]>("live", { source }),
  history: (unit: PeriodUnit, from: string, to: string, source: number | null, route: Route | null) =>
    invoke<PeriodRow[]>("history", { unit, from, to, source, route }),
  /** Traffic of the local days `from` through `to` by route; the empty key is what has none. */
  routes: (from: string, to: string, source: number | null) => invoke<RankRow[]>("routes", { from, to, source }),
  /** Traffic by route over about the last `minutes` (whole quarter hours). */
  recentRoutes: (source: number | null, minutes: number) => invoke<RankRow[]>("recent_routes", { source, minutes }),
  ranking: (kind: DimKind, from: string, to: string, source: number | null, route: Route | null) =>
    invoke<RankRow[]>("ranking", { kind, from, to, source, route }),
  hosts: (from: string, to: string, source: number | null, route: Route | null, process: string | null, group: string | null, view: HostView) =>
    invoke<RankRow[]>("hosts", { from, to, source, route, process, group, view }),
  attributionRules: () => invoke<AttributionRule[]>("attribution_rules"),
  saveAttributionRule: (rule: AttributionRule) => invoke<number>("save_attribution_rule", { rule }),
  deleteAttributionRule: (id: number) => invoke<void>("delete_attribution_rule", { id }),
  hostGroups: () => invoke<HostGroup[]>("host_groups"),
  createGroup: (name: string, note: string) => invoke<number>("create_group", { name, note }),
  updateGroup: (id: number, name: string, note: string) => invoke<void>("update_group", { id, name, note }),
  deleteGroup: (id: number) => invoke<void>("delete_group", { id }),
  /** Entries one per line, read as a text import. */
  addRules: (group: number, text: string) => invoke<{ added: Added; problems: ImportProblem[] }>("add_rules", { group, text }),
  updateRule: (id: number, kind: RuleKind, value: string) => invoke<void>("update_rule", { id, kind, value }),
  deleteRules: (ids: number[]) => invoke<void>("delete_rules", { ids }),
  importPreview: (text: string, format: ImportFormat) => invoke<ImportPreview>("import_preview", { text, format }),
  importApply: (entries: ImportEntry[], defaultGroup: string) => invoke<Added>("import_apply", { entries, defaultGroup }),
  /** The configuration in use, read again when its file was edited outside the app. */
  rulesProfile: () => invoke<RulesProfile>("rules_profile"),
  rulesProfiles: () => invoke<RulesProfileSummary[]>("rules_profiles"),
  switchRulesProfile: (name: string) => invoke<void>("switch_rules_profile", { name }),
  /** A new configuration, empty or a copy of the one in use; returns its name. */
  createRulesProfile: (name: string, copy: boolean) => invoke<string>("create_rules_profile", { name, copy }),
  importRulesProfile: (name: string, text: string) => invoke<string>("import_rules_profile", { name, text }),
  deleteRulesProfile: (name: string) => invoke<void>("delete_rules_profile", { name }),
  openRulesProfile: () => invoke<void>("open_rules_profile"),
  revealRulesProfile: () => invoke<void>("reveal_rules_profile"),
  subscriptions: () => invoke<Subscription[]>("subscriptions"),
  saveCycle: (cycle: CycleConfig) => invoke<number>("save_cycle", { cycle }),
  deleteCycle: (id: number) => invoke<void>("delete_cycle", { id }),
  dataBounds: (source: number | null) => invoke<Bounds | null>("data_bounds", { source }),
  topDays: (source: number | null, route: Route | null, limit: number) => invoke<PeriodRow[]>("top_days", { source, route, limit }),
  /** Opens a save dialog and writes CSV or JSON; false when the user cancelled it. */
  exportFile: (name: string, content: string, kind: "csv" | "json" | "yaml") => invoke<boolean>("export_file", { name, content, kind }),
  releaseNotes: (after: string, upto: string) => invoke<ReleaseNote[]>("release_notes", { after, upto }),
  openReleasePage: (version: string) => invoke<void>("open_release_page", { version }),
  openDataDir: () => invoke<void>("open_data_dir"),
  restartApp: () => invoke<void>("restart_app"),
  quitApp: () => invoke<void>("quit_app"),
  /** Sends a notification now; `dev` is a development build (on macOS, sent as Terminal's). */
  testNotification: () => invoke<{ dev: boolean }>("test_notification"),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
