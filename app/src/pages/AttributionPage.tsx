import { FileCog, Loader2, Plus, Signpost, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Group, Page, PageHeader } from "@/components/PageHeader";
import { RulesFileDialog } from "@/components/RulesFileDialog";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Input, Label, Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type AttributionField, type AttributionRule } from "@/lib/api";
import { useOnShow } from "@/lib/pageActive";
import { isoDate } from "@/lib/utils";

const FIELD_LABEL: Record<AttributionField, string> = { inbound: "入站名称" };
const FIELDS: AttributionField[] = ["inbound"];

/** Inbound names seen over the last 30 days, offered while typing a condition. */
function useSeenInbounds(open: boolean): string[] {
  const [names, setNames] = useState<string[]>([]);
  useEffect(() => {
    if (!open) return;
    const today = new Date();
    const from = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 29);
    api.ranking("inbound", isoDate(from), isoDate(today), null, null).then(
      (rows) => setNames(rows.map((r) => r.key).filter(Boolean)),
      () => setNames([]),
    );
  }, [open]);
  return names;
}

/** Creates a rule, or edits one. */
function RuleDialog({
  open,
  rule,
  nextPriority,
  onClose,
  onSaved,
}: {
  open: boolean;
  rule: AttributionRule | null;
  nextPriority: number;
  onClose: () => void;
  onSaved: () => void;
}) {
  const toast = useToast();
  const [name, setName] = useState("");
  const [field, setField] = useState<AttributionField>("inbound");
  const [value, setValue] = useState("");
  const [priority, setPriority] = useState("");
  const [busy, setBusy] = useState(false);
  const seen = useSeenInbounds(open);

  useEffect(() => {
    if (!open) return;
    setName(rule?.name ?? "");
    setField(rule?.field ?? "inbound");
    setValue(rule?.value ?? "");
    setPriority(String(rule?.priority ?? nextPriority));
  }, [open, rule, nextPriority]);

  if (!open) return null;
  const priorityValue = Number(priority);
  const valid = name.trim() !== "" && value.trim() !== "" && priority.trim() !== "" && Number.isInteger(priorityValue);

  const save = async () => {
    if (!valid) return;
    setBusy(true);
    try {
      await api.saveAttributionRule({ id: rule?.id ?? null, name, field, value, priority: priorityValue, enabled: rule?.enabled ?? true });
      onSaved();
      onClose();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title={rule ? "编辑规则" : "新建规则"}
      description="内核没有识别出进程的连接，符合条件时按这个名称计入进程排行。只影响之后读到的流量。"
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button disabled={busy || !valid} onClick={() => void save()}>
            {busy && <Loader2 className="animate-spin" />}
            保存
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label htmlFor="rule-name">显示名称</Label>
          <Input id="rule-name" autoFocus placeholder="例如 Container: us-workstation" value={name} onChange={(e) => setName(e.target.value)} />
        </div>
        <div className="space-y-1">
          <Label htmlFor="rule-value">匹配条件</Label>
          <div className="flex gap-2">
            <Select aria-label="匹配字段" value={field} onChange={(e) => setField(e.target.value as AttributionField)} className="w-[120px] shrink-0">
              {FIELDS.map((f) => (
                <option key={f} value={f}>
                  {FIELD_LABEL[f]}
                </option>
              ))}
            </Select>
            <span className="flex shrink-0 items-center text-[12.5px] text-muted">等于</span>
            <Input
              id="rule-value"
              list="seen-inbounds"
              className="font-mono"
              placeholder="例如 US-Workstation-Port"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void save()}
            />
            <datalist id="seen-inbounds">
              {seen.map((n) => (
                <option key={n} value={n} />
              ))}
            </datalist>
          </div>
          <p className="text-[11.5px] leading-[17px] text-muted">
            区分大小写，需与内核里的入站名称完全一致（配置里 listeners 的 name，或分布页「入站」里看到的名称）。
          </p>
        </div>
        <div className="space-y-1">
          <Label htmlFor="rule-priority">优先级</Label>
          <Input id="rule-priority" type="number" step={1} className="w-[120px]" value={priority} onChange={(e) => setPriority(e.target.value)} />
          <p className="text-[11.5px] leading-[17px] text-muted">数字小的先匹配；数字相同时，先建的规则优先。</p>
        </div>
      </div>
    </Dialog>
  );
}

function RuleIcon({ off }: { off: boolean }) {
  return (
    <span className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-pill ${off ? "text-faint" : "text-fg-2"}`}>
      <Signpost className="h-4 w-4" />
    </span>
  );
}

export function AttributionPage() {
  const toast = useToast();
  const [rules, setRules] = useState<AttributionRule[] | null>(null);
  const [editing, setEditing] = useState<{ open: boolean; rule: AttributionRule | null }>({ open: false, rule: null });
  const [deleting, setDeleting] = useState<AttributionRule | null>(null);
  const [rulesFile, setRulesFile] = useState(false);

  const load = useCallback(async () => {
    try {
      setRules(await api.attributionRules());
    } catch (e) {
      toast(errorText(e), "error");
    }
  }, [toast]);

  useEffect(() => {
    void load();
  }, [load]);
  useOnShow(load);

  const toggle = async (r: AttributionRule, enabled: boolean) => {
    try {
      await api.saveAttributionRule({ ...r, enabled });
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const remove = async (r: AttributionRule) => {
    try {
      if (r.id !== null) await api.deleteAttributionRule(r.id);
      setDeleting(null);
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const nextPriority = rules?.length ? Math.max(...rules.map((r) => r.priority)) + 10 : 10;
  const create = () => setEditing({ open: true, rule: null });

  return (
    <Page>
      <PageHeader
        title="归属规则"
        subtitle="给内核没有识别出进程的流量起名字，例如容器或局域网设备经专用入站进来的流量。"
        actions={
          <>
            <Button size="sm" variant="outline" onClick={() => setRulesFile(true)}>
              <FileCog />
              配置文件
            </Button>
            <Button size="sm" onClick={create}>
              <Plus />
              新建规则
            </Button>
          </>
        }
      />
      <div className="px-5">
        {rules === null ? (
          <div className="flex items-center gap-2 px-1 text-[12.5px] text-muted">
            <Loader2 className="h-4 w-4 animate-spin" />
            正在读取…
          </div>
        ) : rules.length === 0 ? (
          <div className="rounded-xl border border-dashed border-line px-6 py-10 text-center text-[12.5px] leading-5 text-muted">
            还没有规则。比如给容器单独开了一个入站 US-Workstation-Port，可以把从它进来、内核又认不出进程的流量记为「Container: us-workstation」。
            <div className="mt-4 flex justify-center">
              <Button size="sm" onClick={create}>
                <Plus />
                新建规则
              </Button>
            </div>
          </div>
        ) : (
          <Group label={`规则（${rules.length}）`}>
            {rules.map((r) => (
              <div key={r.id} className="flex items-center gap-3 px-4 py-3">
                <RuleIcon off={!r.enabled} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className={`truncate text-[13.5px] font-medium ${r.enabled ? "" : "text-muted"}`}>{r.name}</span>
                    <Tag>优先级 {r.priority}</Tag>
                    {!r.enabled && <Tag>已停用</Tag>}
                  </div>
                  <div className="truncate text-[11.5px] text-muted">
                    {FIELD_LABEL[r.field]} = <span className="font-mono">{r.value}</span>
                  </div>
                </div>
                <Switch checked={r.enabled} onCheckedChange={(v) => void toggle(r, v)} label={`${r.enabled ? "停用" : "启用"}规则 ${r.name}`} />
                <Button size="sm" variant="outline" onClick={() => setEditing({ open: true, rule: r })}>
                  编辑
                </Button>
                <Button size="iconSm" variant="ghost" aria-label={`删除规则 ${r.name}`} title="删除" onClick={() => setDeleting(r)}>
                  <Trash2 />
                </Button>
              </div>
            ))}
          </Group>
        )}
        <p className="mt-5 px-1 text-[11.5px] leading-relaxed text-muted">
          规则只用于内核读到了连接、但没有给出进程名的流量；内核识别出的进程名始终保留，也不会根据 127.0.0.1 或访问的域名去猜是哪个程序。
          命中规则的流量在分布页的进程排行和它的域名明细里以规则的名称出现，只是换了归属，总量不变。
          新建或修改规则只影响之后读到的流量，已经记录的不会改写。
        </p>
      </div>
      <RuleDialog
        open={editing.open}
        rule={editing.rule}
        nextPriority={nextPriority}
        onClose={() => setEditing({ open: false, rule: null })}
        onSaved={() => void load()}
      />
      <RulesFileDialog open={rulesFile} onClose={() => setRulesFile(false)} onDone={() => void load()} />
      <Dialog
        open={deleting !== null}
        onClose={() => setDeleting(null)}
        title="删除规则"
        description={deleting ? `删除「${deleting.name}」后，之后读到的同类流量会计入「进程未识别」；已经记录的流量仍以这个名称显示。` : undefined}
        footer={
          <>
            <Button variant="ghost" onClick={() => setDeleting(null)}>
              取消
            </Button>
            <Button variant="destructive" onClick={() => deleting && void remove(deleting)}>
              删除
            </Button>
          </>
        }
      />
    </Page>
  );
}
