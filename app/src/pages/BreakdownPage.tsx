import { ChevronRight, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Page, PageHeader } from "@/components/PageHeader";
import { Segmented } from "@/components/Segmented";
import { Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api, errorText, type DimKind, type RankRow, type Source } from "@/lib/api";
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
function keyLabel(tab: Tab, key: string): { text: string; muted: boolean } {
  if (!key) {
    const text = { outbound: "未归类", subscription: "未归类", process: "未知进程", inbound: "未知入站", host: "其他" }[tab];
    return { text, muted: true };
  }
  if (key === "DIRECT" && (tab === "outbound" || tab === "subscription")) return { text: "直连（DIRECT）", muted: false };
  return { text: key, muted: false };
}

const HINTS: Record<Tab, string> = {
  outbound: "实际出站的节点（连接链路的第一跳）。「未归类」是连接在两次读取之间就已关闭、看不到去向的流量，以及开始按连接统计之前的流量。",
  subscription:
    "节点来自哪个订阅：配置里用了 proxy-providers 时取 provider 名；Clash Verge Rev 取当时启用的配置名。直连不属于任何订阅。",
  process: "发起连接的程序。内核需要能查到进程：TUN 模式下通常默认可以，否则在配置中设置 find-process-mode: always。",
  inbound: "流量从内核的哪个入口进来，例如混合端口、TUN，或为容器、局域网设备单独开的端口。",
  host: "访问的域名（没有域名时是 IP）。今天的数据完整；过去的每一天只保留用量最高的 200 个，其余计入「其他」。",
};

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
}: {
  tab: Tab;
  row: RankRow;
  total: number;
  max: number;
  expandable?: boolean;
  open?: boolean;
  onToggle?: () => void;
  indent?: boolean;
}) {
  const v = row.up + row.down;
  const { text, muted } = keyLabel(tab, row.key);
  const Tag = expandable ? "button" : "div";
  return (
    <Tag
      onClick={expandable ? onToggle : undefined}
      aria-expanded={expandable ? open : undefined}
      title={`下载 ${bytes(row.down)} · 上传 ${bytes(row.up)}`}
      className={cn(
        "grid w-full grid-cols-[minmax(0,1fr)_minmax(80px,30%)_5.5rem_3.25rem] items-center gap-3 px-4 py-1.5 text-left text-[12.5px] active:!scale-100",
        expandable && "hover:bg-pill/60",
        indent && "py-1 text-[12px]",
      )}
    >
      <span className={cn("flex min-w-0 items-center gap-1", indent && "pl-5")}>
        {expandable && <ChevronRight className={cn("h-3.5 w-3.5 shrink-0 text-muted transition-transform", open && "rotate-90")} />}
        <span className={cn("selectable truncate", muted ? "text-muted" : "text-fg", indent && "text-fg-2")}>{text}</span>
      </span>
      <Bar value={v} max={max} />
      <span className="text-right font-mono tabular-nums">{bytes(v)}</span>
      <span className="text-right font-mono tabular-nums text-muted">{total > 0 ? `${((v / total) * 100).toFixed(1)}%` : "—"}</span>
    </Tag>
  );
}

function ProcessHosts({ process, from, to, source, merge }: { process: string; from: string; to: string; source: number | null; merge: boolean }) {
  const [rows, setRows] = useState<RankRow[] | null>(null);
  useEffect(() => {
    let active = true;
    api.hosts(from, to, source, process, merge).then(
      (r) => active && setRows(r),
      () => active && setRows([]),
    );
    return () => {
      active = false;
    };
  }, [process, from, to, source, merge]);
  if (!rows) return <div className="px-4 py-1.5 pl-9 text-[12px] text-muted">正在读取…</div>;
  if (!rows.length) return <div className="px-4 py-1.5 pl-9 text-[12px] text-muted">没有记录到域名</div>;
  const total = rows.reduce((t, r) => t + r.up + r.down, 0);
  const max = Math.max(...rows.map((r) => r.up + r.down));
  return (
    <div className="bg-card-2 py-1">
      {rows.slice(0, 15).map((r) => (
        <Row key={r.key} tab="host" row={r} total={total} max={max} indent />
      ))}
    </div>
  );
}

export function BreakdownPage({ sources }: { sources: Source[] | null }) {
  const [tab, setTab] = useState<Tab>("outbound");
  const [range, setRange] = useState<RangeId>("today");
  const [source, setSource] = useState<number | null>(null);
  const [merge, setMerge] = useState(true);
  const [weekStartsSunday, setWeekStartsSunday] = useState(false);
  const [rows, setRows] = useState<RankRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [all, setAll] = useState(false);
  const [open, setOpen] = useState<string | null>(null);

  const recorded = (sources ?? []).filter((s): s is Source & { id: number } => s.id !== null);
  const [from, to] = useMemo(() => rangeDates(range, weekStartsSunday), [range, weekStartsSunday]);

  useEffect(() => {
    void api.getSettings().then((s) => setWeekStartsSunday(s.weekStartsSunday)).catch(() => {});
  }, []);

  const load = useCallback(async () => {
    try {
      setRows(tab === "host" ? await api.hosts(from, to, source, null, merge) : await api.ranking(tab, from, to, source));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, [tab, from, to, source, merge]);

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
          recorded.length > 1 && (
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
          )
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
                <label className="ml-auto flex items-center gap-2">
                  按主域名合并
                  <Switch label="按主域名合并" checked={merge} onCheckedChange={setMerge} />
                </label>
              )}
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
                {shown.map((r) => (
                  <div key={r.key}>
                    <Row
                      tab={tab}
                      row={r}
                      total={total}
                      max={max}
                      expandable={tab === "process"}
                      open={open === r.key}
                      onToggle={() => setOpen((o) => (o === r.key ? null : r.key))}
                    />
                    {tab === "process" && open === r.key && <ProcessHosts process={r.key} from={from} to={to} source={source} merge={merge} />}
                  </div>
                ))}
                {!all && (rows?.length ?? 0) > SHOWN && (
                  <button className="w-full px-4 py-2 text-left text-[12px] text-muted hover:text-fg active:!scale-100" onClick={() => setAll(true)}>
                    显示全部 {rows?.length} 项
                  </button>
                )}
              </div>
            )}
          </div>
        </section>
        <p className="px-1 text-[11.5px] leading-relaxed text-muted">{HINTS[tab]}</p>
      </div>
    </Page>
  );
}
