import { CalendarClock, Loader2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { CycleDialog } from "@/components/CycleDialog";
import { Page, PageHeader } from "@/components/PageHeader";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { api, errorText, type CycleView, type Reported, type Subscription } from "@/lib/api";
import { dateOf, describeReset, meterTone, percentUsed, remaining, shortTime } from "@/lib/cycle";
import { bytes, bytesShort, cn } from "@/lib/utils";

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="min-w-0 bg-card px-4 py-3">
      <div className="truncate font-mono text-[17px] font-semibold tabular-nums tracking-[-0.01em]">{value}</div>
      <div className="mt-0.5 truncate text-[11.5px] text-muted">{label}</div>
      {sub && <div className="truncate text-[10.5px] text-muted">{sub}</div>}
    </div>
  );
}

function Meter({ c }: { c: CycleView }) {
  const pct = percentUsed(c);
  return (
    <div className="h-1.5 overflow-hidden rounded-full bg-pill">
      <div className={cn("h-full rounded-full", meterTone(pct))} style={{ width: `${pct === null ? 0 : Math.min(100, Math.max(pct, 1))}%` }} />
    </div>
  );
}

function ReportedLine({ r }: { r: Reported }) {
  const used = r.upload + r.download;
  const parts = [
    `已用 ${bytes(used)}${r.total ? ` / ${bytes(r.total)}` : ""}`,
    r.total ? `剩余 ${bytes(Math.max(0, r.total - used))}` : "",
    r.expire ? `到期 ${dateOf(r.expire)}` : "",
    r.updated ? `更新于 ${shortTime(r.updated)}` : "",
  ].filter(Boolean);
  return (
    <div className="flex flex-wrap items-baseline gap-x-2 text-[12px] text-fg-2">
      <span className="text-muted">机场显示</span>
      {parts.map((p, i) => (
        <span key={i} className={i === 0 ? "font-mono tabular-nums text-fg" : ""}>
          {p}
        </span>
      ))}
    </div>
  );
}

function CycleBody({ c, now }: { c: CycleView; now: number }) {
  const pct = percentUsed(c);
  const days = Math.max(1, (Math.min(now, c.end) - c.start) / 86400);
  return (
    <div className="space-y-3 px-4 pb-3">
      <div>
        <div className="mb-1.5 flex items-baseline justify-between gap-3 text-[12px]">
          <span className="text-muted">
            {shortTime(c.start)} – {shortTime(c.end)} · 还剩 {remaining(c.end, now)}
          </span>
          <span className="font-mono tabular-nums">{pct === null ? "未设配额" : `${pct.toFixed(1)}%`}</span>
        </div>
        <Meter c={c} />
      </div>
      <div className="grid grid-cols-2 gap-px overflow-hidden rounded-xl border border-line-2 bg-line-2 min-[620px]:grid-cols-4">
        <Stat label="本周期已用" value={bytes(c.used)} sub={c.mode === "downloadOnly" ? "仅计下载" : `↓ ${bytesShort(c.bytes.down)} · ↑ ${bytesShort(c.bytes.up)}`} />
        <Stat label="配额" value={c.quota ? bytes(c.quota) : "—"} sub={c.quota ? `剩余 ${bytes(Math.max(0, c.quota - c.used))}` : "未设置"} />
        <Stat label="预计周期末" value={c.projected === null ? "—" : bytes(c.projected)} sub={c.projected === null ? "周期过一天后估算" : "按目前的速度"} />
        <Stat label="日均" value={bytes(c.used / days)} sub="按本周期已过去的天数" />
      </div>
    </div>
  );
}

export function SubscriptionsPage() {
  const [subs, setSubs] = useState<Subscription[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Subscription | null>(null);
  const now = Math.floor(Date.now() / 1000);

  const load = useCallback(async () => {
    try {
      setSubs(await api.subscriptions());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = window.setInterval(load, 30_000);
    return () => window.clearInterval(timer);
  }, [load]);

  return (
    <Page>
      <PageHeader title="订阅" subtitle="按订阅的计费周期统计用量；设置配额后，用到提醒线时发系统通知。" />
      <div className="space-y-5 px-5">
        {error && <p className="selectable text-[12px] text-danger">{error}</p>}
        {subs === null ? (
          <div className="flex items-center gap-2 px-1 text-[12.5px] text-muted">
            <Loader2 className="h-4 w-4 animate-spin" />
            正在读取…
          </div>
        ) : subs.length === 0 ? (
          <div className="rounded-xl border border-dashed border-line px-6 py-10 text-center text-[12.5px] leading-5 text-muted">
            还没有发现订阅。使用 Clash Verge Rev（按启用的配置区分订阅），或在配置中用 proxy-providers 引用订阅后，有流量经过的订阅会出现在这里。
          </div>
        ) : (
          subs.map((s) => (
            <section key={s.name} className="list-card">
              <div className="flex items-center gap-3 px-4 py-3">
                <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-pill text-fg-2">
                  <CalendarClock className="h-3.5 w-3.5" />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="truncate text-[13.5px] font-medium">{s.name}</span>
                    {s.cycle && <Tag>{describeReset(s.cycle.reset)}</Tag>}
                    {s.cycle?.quota && percentUsed(s.cycle)! >= 100 && <Tag tone="danger">已超出配额</Tag>}
                  </div>
                  {!s.cycle && <div className="text-[11.5px] text-muted">本月 {bytes(s.month.up + s.month.down)}</div>}
                </div>
                <Button size="sm" variant={s.cycle ? "ghost" : "outline"} onClick={() => setEditing(s)}>
                  {s.cycle ? "修改" : "设置计费周期"}
                </Button>
              </div>
              {s.cycle && <CycleBody c={s.cycle} now={now} />}
              {s.reported && (
                <div className="px-4 py-2.5">
                  <ReportedLine r={s.reported} />
                </div>
              )}
            </section>
          ))
        )}
        <p className="px-1 text-[11.5px] leading-relaxed text-muted">
          这里的用量是经过本机（及手动添加的内核）的流量，按订阅区分：配置了 proxy-providers 时按 provider，Clash Verge Rev
          按当时启用的配置。机场显示的已用量来自订阅信息，包含使用同一订阅的所有设备，并可能按节点倍率计费，所以两者通常不同。开始按连接统计之前的流量无法区分订阅，不计入周期。
        </p>
      </div>
      <CycleDialog subscription={editing} onClose={() => setEditing(null)} onSaved={() => void load()} />
    </Page>
  );
}
