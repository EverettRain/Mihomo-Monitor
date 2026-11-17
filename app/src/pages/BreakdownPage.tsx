import { ChevronRight, FolderPlus, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { AddToGroupDialog } from "@/components/AddToGroupDialog";
import { Page, PageHeader } from "@/components/PageHeader";
import { RouteShare } from "@/components/RouteShare";
import { Segmented } from "@/components/Segmented";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { Select } from "@/components/ui/input";
import { api, errorText, type DimKind, type HostView, type RankRow, type Route, type Source } from "@/lib/api";
import { ROUTE_FILTERS, routeOf, useRouteFilter } from "@/lib/route";
import { bytes, cn, isoDate } from "@/lib/utils";

type Tab = DimKind | "host";

const TABS: readonly (readonly [Tab, string])[] = [
  ["outbound", "节点"],
  ["subscription", "订阅"],
  ["process", "进程"],
  ["inbound", "入站"],
  ["host", "域名"],
];

type RangeId = "today" | "yesterday" | "week" | "month" | "last30" | "year";
const RANGES: readonly (readonly [RangeId, string])[] = [
  ["today", "今天"],
  ["yesterday", "昨天"],
  ["week", "本周"],
  ["month", "本月"],
  ["last30", "近 30 天"],
  ["year", "今年"],
];

/** Rows shown before "显示全部". */
const SHOWN = 30;

function rangeDates(id: RangeId, weekStartsSunday: boolean): [string, string] {
  const now = new Date();
  const day = (offset: number) => new Date(now.getFullYear(), now.getMonth(), now.getDate() + offset);
  switch (id) {
    case "today":
      return [isoDate(day(0)), isoDate(day(0))];
    case "yesterday":
      return [isoDate(day(-1)), isoDate(day(-1))];
    case "week": {
      const back = weekStartsSunday ? now.getDay() : (now.getDay() + 6) % 7;
      return [isoDate(day(-back)), isoDate(day(0))];
    }
    case "month":
      return [isoDate(new Date(now.getFullYear(), now.getMonth(), 1)), isoDate(day(0))];
    case "last30":
      return [isoDate(day(-29)), isoDate(day(0))];
    case "year":
      return [isoDate(new Date(now.getFullYear(), 0, 1)), isoDate(day(0))];
  }
}

/** How a key reads in the list; the empty key and DIRECT get words instead. */
function keyLabel(tab: Tab, row: RankRow): { text: string; muted: boolean } {
  const key = row.key;
  if (row.uncollected) return { text: "未采到连接明细", muted: true };
  if (row.unrouted) return { text: "未区分是否经代理", muted: true };
  if (!key) {
    const text = { outbound: "未归类", subscription: "未归类", process: "进程未识别", inbound: "未知入站", host: "其他" }[tab];
    return { text, muted: true };
  }
  if (key === "DIRECT" && (tab === "outbound" || tab === "subscription")) return { text: "直连（DIRECT）", muted: false };
  return { text: key, muted: false };
}

const HINTS: Record<Tab, string> = {
  outbound: "实际出站的节点（连接链路的第一跳）。「未归类」是连接在两次读取之间就已关闭、看不到去向的流量，以及开始按连接统计之前的流量。",
  subscription:
    "节点来自哪个订阅：配置里用了 proxy-providers 时取 provider 名；Clash Verge Rev 取当时启用的配置名。直连不属于任何订阅。",
  process:
    "发起连接的程序。内核需要能查到进程：TUN 模式下通常默认可以，否则在配置中设置 find-process-mode: always。「进程未识别」是读到了连接、但内核没有给出进程名且没有命中归属规则的流量，展开可看它访问的域名；「未采到连接明细」是内核总量中没有被任何读到的连接计入的部分，例如两次读取之间就已关闭的连接。标有「规则」的是由「归属规则」页的规则命名的流量。",
  inbound: "流量从内核的哪个入口进来，例如混合端口、TUN，或为容器、局域网设备单独开的端口。",
  host: "访问的域名（没有域名时是 IP）。今天的数据完整；过去的每一天只保留用量最高的 200 个，其余计入「其他」。按分组查看时，属于同一分组的域名与 IP 合并为分组名称，分组在「域名分组」页管理；点行末的按钮可以把一个域名加入分组。",
};

const VIEWS: readonly (readonly [HostView, string])[] = [
  ["group", "按分组"],
  ["domain", "按主域名"],
  ["host", "原始"],
];

function Bar({ value, max }: { value: number; max: number }) {
  return (
    <div className="h-1.5 overflow-hidden rounded-full bg-pill">
      <div className="h-full rounded-full bg-accent" style={{ width: `${Math.max((value / Math.max(max, 1)) * 100, value > 0 ? 1 : 0)}%` }} />
    </div>
  );
}

function Row({
  tab,
  row,
  total,
  max,
  expandable,
  open,
  onToggle,
  indent,
  onAdd,
  rule,
}: {
  tab: Tab;
  row: RankRow;
  /** The process name comes from an attribution rule. */
  rule?: boolean;
  total: number;
  max: number;
  expandable?: boolean;
  open?: boolean;
  onToggle?: () => void;
  indent?: boolean;
  /** Offers to put this host into a group. */
  onAdd?: () => void;
}) {
  const v = row.up + row.down;
  const { text, muted } = keyLabel(tab, row);
  return (
    <div
      role={expandable ? "button" : undefined}
      tabIndex={expandable ? 0 : undefined}
      onClick={expandable ? onToggle : undefined}
      onKeyDown={expandable ? (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), onToggle?.()) : undefined}
      aria-expanded={expandable ? open : undefined}
      title={`下载 ${bytes(row.down)} · 上传 ${bytes(row.up)}`}
      className={cn(
        "group grid w-full grid-cols-[minmax(0,1fr)_minmax(80px,28%)_5.5rem_3.25rem_1.75rem] items-center gap-3 py-1.5 pl-4 pr-2 text-left text-[12.5px] outline-none",
        expandable && "cursor-pointer hover:bg-pill/60 focus-visible:bg-pill/60",
        indent && "py-1 text-[12px]",
      )}
    >
      <span className={cn("flex min-w-0 items-center gap-1.5", indent && "pl-5")}>
        {expandable && <ChevronRight className={cn("h-3.5 w-3.5 shrink-0 text-muted transition-transform", open && "rotate-90")} />}
        <span className={cn("selectable truncate", muted ? "text-muted" : "text-fg", indent && !row.group && "text-fg-2")}>{text}</span>
        {row.group && <Tag>分组</Tag>}
        {rule && <Tag>规则</Tag>}
      </span>
      <Bar value={v} max={max} />
      <span className="text-right font-mono tabular-nums">{bytes(v)}</span>
      <span className="text-right font-mono tabular-nums text-muted">{total > 0 ? `${((v / total) * 100).toFixed(1)}%` : "—"}</span>
      <span className="flex h-[18px] items-center justify-end">
        {onAdd && row.key && !row.group && (
          <Button
            variant="ghost"
            size="iconSm"
            aria-label={`把 ${row.key} 加入分组`}
            title="加入分组"
            className="h-[22px] w-[22px] opacity-0 focus-visible:opacity-100 group-hover:opacity-100 [&_svg]:h-3.5 [&_svg]:w-3.5"
            onClick={(e) => {
              e.stopPropagation();
              onAdd();
            }}
          >
            <FolderPlus />
          </Button>
        )}
      </span>
    </div>
  );
}

/** Hosts under an expanded row: a process's hosts, or a group's members. */
function SubHosts({
  from,
  to,
  source,
  route,
  process,
  group,
  view,
  onAdd,
  version,
}: {
  from: string;
  to: string;
  source: number | null;
  route: Route | null;
  process: string | null;
  group: string | null;
  view: HostView;
  onAdd: (host: string) => void;
  /** Changes when groups change, to read again. */
  version: number;
}) {
  const [rows, setRows] = useState<RankRow[] | null>(null);
  useEffect(() => {
    let active = true;
    api.hosts(from, to, source, route, process, group, view).then(
      (r) => active && setRows(r),
      () => active && setRows([]),
    );
    return () => {
      active = false;
    };
  }, [process, group, from, to, source, route, view, version]);
  if (!rows) return <div className="px-4 py-1.5 pl-9 text-[12px] text-muted">正在读取…</div>;
  if (!rows.length) return <div className="px-4 py-1.5 pl-9 text-[12px] text-muted">没有记录到域名</div>;
  const total = rows.reduce((t, r) => t + r.up + r.down, 0);
  const max = Math.max(...rows.map((r) => r.up + r.down));
  return (
    <div className="bg-card-2 py-1">
      {rows.slice(0, 15).map((r) => (
        <Row key={`${r.group}${r.key}`} tab="host" row={r} total={total} max={max} indent onAdd={() => onAdd(r.key)} />
      ))}
    </div>
  );
}

export function BreakdownPage({ sources }: { sources: Source[] | null }) {
  const [tab, setTab] = useState<Tab>("outbound");
  const [range, setRange] = useState<RangeId>("today");
  const [source, setSource] = useState<number | null>(null);
  const [view, setView] = useState<HostView>("group");
  const [adding, setAdding] = useState<string | null>(null);
  const [version, setVersion] = useState(0);
  const [weekStartsSunday, setWeekStartsSunday] = useState(false);
  const [rows, setRows] = useState<RankRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [all, setAll] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [route, setRoute] = useRouteFilter();
  const [ruleNames, setRuleNames] = useState<Set<string>>(new Set());

  const recorded = (sources ?? []).filter((s): s is Source & { id: number } => s.id !== null);
  const [from, to] = useMemo(() => rangeDates(range, weekStartsSunday), [range, weekStartsSunday]);

  useEffect(() => {
    void api.getSettings().then((s) => setWeekStartsSunday(s.weekStartsSunday)).catch(() => {});
  }, []);

  const load = useCallback(async () => {
    try {
      const r = routeOf(route);
      setRows(tab === "host" ? await api.hosts(from, to, source, r, null, null, view) : await api.ranking(tab, from, to, source, r));
      if (tab === "process") setRuleNames(new Set((await api.attributionRules()).map((r) => r.name)));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, [tab, from, to, source, view]);

  useEffect(() => {
    setRows(null);
    setAll(false);
    setOpen(null);
    void load();
    const timer = window.setInterval(load, 30_000);
    return () => window.clearInterval(timer);
  }, [load]);

  const total = (rows ?? []).reduce((t, r) => t + r.up + r.down, 0);
  const max = Math.max(1, ...(rows ?? []).map((r) => r.up + r.down));
  const shown = all ? (rows ?? []) : (rows ?? []).slice(0, SHOWN);
  const onlyUnknown = tab === "process" && rows !== null && rows.length > 0 && rows.every((r) => !r.key);

  return (
    <Page>
      <PageHeader
        title="分布"
        subtitle="流量去了哪里：按节点、订阅、进程、入站和域名排行。"
        actions={
          <>
            <Segmented label="流量范围" value={route} onChange={setRoute} options={ROUTE_FILTERS} />
            {recorded.length > 1 && (
              <Select
                aria-label="查看的内核"
                value={source ?? ""}
                onChange={(e) => setSource(e.target.value ? Number(e.target.value) : null)}
                className="h-7 w-auto min-w-[120px] max-w-[200px] truncate text-[12.5px]"
              >
                <option value="">全部内核</option>
                {recorded.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
              </Select>
            )}
          </>
        }
      />
      <div className="space-y-5 px-5">
        <section className="list-card">
          <div className="flex flex-wrap items-center justify-between gap-3 px-4 pt-3">
            <Segmented label="维度" value={tab} onChange={setTab} options={TABS} />
            <Segmented label="时间范围" value={range} onChange={setRange} options={RANGES} />
          </div>
          <div className="border-t-0 pb-2 pt-3">
            <div className="mb-1.5 flex flex-wrap items-center gap-x-5 gap-y-1 px-4 text-[12px] text-muted">
              <span>
                合计<span className="ml-1.5 font-mono text-[15px] font-semibold tabular-nums text-fg">{rows ? bytes(total) : "—"}</span>
              </span>
              {rows && <span>{rows.filter((r) => r.key).length} 项</span>}
              {error && <span className="text-danger">{error}</span>}
              {tab === "host" && (
                <span className="ml-auto">
                  <Segmented label="域名的显示方式" value={view} onChange={setView} options={VIEWS} />
                </span>
              )}
            </div>
            <div className="mb-1.5 px-4">
              <RouteShare from={from} to={to} source={source} filter={route} version={rows} />
            </div>
            {onlyUnknown && (
              <p className="mx-4 mb-2 rounded-lg bg-warn-soft px-3 py-2 text-[12px] leading-[18px] text-warn">
                内核没有提供进程信息。开启 TUN 模式，或在配置中设置 find-process-mode: always 后，这里会按程序区分。
              </p>
            )}
            {rows === null ? (
              <div className="flex items-center justify-center gap-2 py-8 text-[12px] text-muted">
                <Loader2 className="h-4 w-4 animate-spin" />
                正在读取…
              </div>
            ) : rows.length === 0 ? (
              <p className="py-8 text-center text-[12px] text-muted">这段时间没有记录</p>
            ) : (
              <div>
                {shown.map((r) => {
                  const id = `${r.group ? "g:" : ""}${r.uncollected ? "u:" : ""}${r.unrouted ? "r:" : ""}${r.key}`;
                  const expands = (tab === "process" && !r.uncollected && !r.unrouted) || (tab === "host" && !!r.group);
                  return (
                    <div key={id}>
                      <Row
                        tab={tab}
                        row={r}
                        total={total}
                        max={max}
                        expandable={expands}
                        open={open === id}
                        onToggle={() => setOpen((o) => (o === id ? null : id))}
                        onAdd={tab === "host" ? () => setAdding(r.key) : undefined}
                        rule={tab === "process" && !!r.key && ruleNames.has(r.key)}
                      />
                      {expands && open === id && (
                        <SubHosts
                          from={from}
                          to={to}
                          source={source}
                          route={routeOf(route)}
                          process={tab === "process" ? r.key : null}
                          group={tab === "host" ? r.key : null}
                          view={tab === "host" ? "host" : view}
                          onAdd={setAdding}
                          version={version}
                        />
                      )}
                    </div>
                  );
                })}
                {!all && (rows?.length ?? 0) > SHOWN && (
                  <button className="w-full px-4 py-2 text-left text-[12px] text-muted hover:text-fg active:!scale-100" onClick={() => setAll(true)}>
                    显示全部 {rows?.length} 项
                  </button>
                )}
              </div>
            )}
          </div>
        </section>
        <p className="px-1 text-[11.5px] leading-relaxed text-muted">
          {HINTS[tab]}
          {route !== "all" &&
            " 只看经代理或直连时，各维度只含能区分的流量；「未区分是否经代理」是开始按经代理 / 直连记录之前的明细，域名明细里不含这部分。"}
        </p>
      </div>
      <AddToGroupDialog
        host={adding}
        onClose={() => setAdding(null)}
        onDone={() => {
          setVersion((v) => v + 1);
          void load();
        }}
      />
    </Page>
  );
}
