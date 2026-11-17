import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Segmented } from "@/components/Segmented";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Input, Label, Select } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type CountMode, type CycleConfig, type Subscription } from "@/lib/api";
import { GB } from "@/lib/cycle";
import { bytes, cn, isoDate } from "@/lib/utils";

const ALERT_CHOICES = [50, 80, 90, 100];
const DAYS = Array.from({ length: 31 }, (_, i) => i + 1);
const TIMES = Array.from({ length: 96 }, (_, i) => [Math.floor(i / 4), (i % 4) * 15] as const);
const pad = (n: number) => String(n).padStart(2, "0");

/** A new subscription's defaults: reset on the day the provider says it expires (most reset monthly on the purchase day), its total as quota. */
function defaults(s: Subscription): CycleConfig {
  const expire = s.reported?.expire ? new Date(s.reported.expire * 1000) : null;
  return {
    id: null,
    subscription: s.name,
    reset: { kind: "monthly", day: expire ? expire.getDate() : 1, hour: 0, minute: 0 },
    quota: s.reported?.total ? s.reported.total : null,
    mode: "both",
    alerts: [80, 100],
  };
}

/** Sets up or edits a subscription's billing cycle. */
export function CycleDialog({ subscription, onClose, onSaved }: { subscription: Subscription | null; onClose: () => void; onSaved: () => void }) {
  const toast = useToast();
  const [c, setC] = useState<CycleConfig | null>(null);
  const [quotaText, setQuotaText] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!subscription) return;
    const initial = subscription.cycle ?? defaults(subscription);
    setC({ id: initial.id, subscription: initial.subscription, reset: initial.reset, quota: initial.quota, mode: initial.mode, alerts: initial.alerts });
    setQuotaText(initial.quota ? String(+(initial.quota / GB).toFixed(2)) : "");
  }, [subscription]);

  if (!subscription || !c) return null;
  const reported = subscription.reported;
  const r = c.reset;
  const time = `${r.hour}:${r.minute}`;
  const setTime = (v: string) => {
    const [hour, minute] = v.split(":").map(Number);
    setC({ ...c, reset: { ...r, hour, minute } });
  };
  const quotaValid = quotaText.trim() === "" || (Number(quotaText) > 0 && Number.isFinite(Number(quotaText)));

  const save = async () => {
    setBusy(true);
    try {
      const quota = quotaText.trim() ? Math.round(Number(quotaText) * GB) : null;
      await api.saveCycle({ ...c, quota });
      onSaved();
      onClose();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (c.id === null) return;
    setBusy(true);
    try {
      await api.deleteCycle(c.id);
      onSaved();
      onClose();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const timeSelect = (
    <Select aria-label="重置时间" value={time} onChange={(e) => setTime(e.target.value)} className="h-8 w-[96px]">
      {TIMES.map(([h, m]) => (
        <option key={`${h}:${m}`} value={`${h}:${m}`}>
          {pad(h)}:{pad(m)}
        </option>
      ))}
    </Select>
  );

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title={`${subscription.name} 的计费周期`}
      description="按周期统计这个订阅的用量；设置配额后，用到提醒线时会发系统通知。"
      className="max-w-[480px]"
      footer={
        <>
          {c.id !== null && (
            <Button variant="ghost" className="mr-auto text-danger hover:text-danger" disabled={busy} onClick={() => void remove()}>
              删除周期
            </Button>
          )}
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button disabled={busy || !quotaValid} onClick={() => void save()}>
            {busy && <Loader2 className="animate-spin" />}
            保存
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <div className="space-y-1.5">
          <Label>重置</Label>
          <Segmented
            label="重置方式"
            value={r.kind}
            onChange={(kind) =>
              setC({
                ...c,
                reset:
                  kind === "monthly"
                    ? { kind, day: 1, hour: r.hour, minute: r.minute }
                    : { kind, start: isoDate(new Date()), days: 30, hour: r.hour, minute: r.minute },
              })
            }
            options={[
              ["monthly", "每月"],
              ["everyDays", "每隔 N 天"],
            ]}
          />
          {r.kind === "monthly" ? (
            <div className="flex items-center gap-2 text-[12.5px] text-fg-2">
              每月
              <Select aria-label="重置日" value={r.day} onChange={(e) => setC({ ...c, reset: { ...r, day: Number(e.target.value) } })} className="h-8 w-[72px]">
                {DAYS.map((d) => (
                  <option key={d} value={d}>
                    {d}
                  </option>
                ))}
              </Select>
              日{timeSelect}
            </div>
          ) : (
            <div className="flex flex-wrap items-center gap-2 text-[12.5px] text-fg-2">
              从
              <Input type="date" aria-label="起始日期" value={r.start} onChange={(e) => setC({ ...c, reset: { ...r, start: e.target.value } })} className="h-8 w-[150px]" />
              起，每
              <Input
                type="number"
                min={1}
                max={366}
                aria-label="周期天数"
                value={r.days}
                onChange={(e) => setC({ ...c, reset: { ...r, days: Math.max(1, Math.min(366, Number(e.target.value) || 1)) } })}
                className="h-8 w-[72px]"
              />
              天{timeSelect}
            </div>
          )}
          <p className="text-[11.5px] leading-[17px] text-muted">
            {r.kind === "monthly"
              ? "重置日大于当月天数时（例如 31 日遇到 30 天的月份），在当月最后一天重置。"
              : "适合按固定天数计费的套餐。"}
            {reported?.expire ? ` 机场显示的到期时间是 ${new Date(reported.expire * 1000).toLocaleString("zh-CN")}。` : ""}
          </p>
        </div>

        <div className="space-y-1.5">
          <Label>计量</Label>
          <Segmented<CountMode>
            label="计量方式"
            value={c.mode}
            onChange={(mode) => setC({ ...c, mode })}
            options={[
              ["both", "上传 + 下载"],
              ["downloadOnly", "仅下载"],
            ]}
          />
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="cycle-quota">配额（GB）</Label>
          <div className="flex items-center gap-2">
            <Input id="cycle-quota" inputMode="decimal" placeholder="不设配额可留空" value={quotaText} onChange={(e) => setQuotaText(e.target.value)} className="w-[160px]" />
            {reported && reported.total > 0 && (
              <Button size="sm" variant="outline" onClick={() => setQuotaText(String(+(reported.total / GB).toFixed(2)))}>
                用机场的总量（{bytes(reported.total)}）
              </Button>
            )}
          </div>
          {!quotaValid && <p className="text-[11.5px] text-danger">请输入大于 0 的数字。</p>}
        </div>

        <div className="space-y-1.5">
          <Label>用到配额的这些比例时提醒</Label>
          <div className="flex gap-1.5">
            {ALERT_CHOICES.map((a) => {
              const on = c.alerts.includes(a);
              return (
                <button
                  key={a}
                  role="checkbox"
                  aria-checked={on}
                  disabled={!quotaText.trim()}
                  onClick={() => setC({ ...c, alerts: on ? c.alerts.filter((x) => x !== a) : [...c.alerts, a].sort((x, y) => x - y) })}
                  className={cn(
                    "h-7 rounded-lg px-3 font-mono text-[12px] tabular-nums disabled:opacity-40",
                    on ? "bg-fg text-card" : "bg-pill text-fg-2 hover:bg-pill-hover",
                  )}
                >
                  {a}%
                </button>
              );
            })}
          </div>
          {!quotaText.trim() && <p className="text-[11.5px] text-muted">设置配额后才能提醒。</p>}
        </div>
      </div>
    </Dialog>
  );
}
