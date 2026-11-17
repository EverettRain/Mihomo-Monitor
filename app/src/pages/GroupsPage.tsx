import { Download, FolderPlus, Loader2, Pencil, Plus, Search, Trash2, Upload } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ImportDialog } from "@/components/ImportDialog";
import { Group, Page, PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Input, Label, Select, Textarea } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type HostGroup, type HostRule, type RuleKind } from "@/lib/api";
import { KIND_LABEL, KINDS } from "@/lib/hostgroup";
import { cn } from "@/lib/utils";

/** Creates a group, or renames one and edits its note. */
function GroupDialog({ group, open, onClose, onSaved }: { group: HostGroup | null; open: boolean; onClose: () => void; onSaved: (id: number) => void }) {
  const toast = useToast();
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!open) return;
    setName(group?.name ?? "");
    setNote(group?.note ?? "");
  }, [open, group]);
  if (!open) return null;
  const save = async () => {
    setBusy(true);
    try {
      if (group) {
        await api.updateGroup(group.id, name, note);
        onSaved(group.id);
      } else {
        onSaved(await api.createGroup(name, note));
      }
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
      title={group ? "编辑分组" : "新建分组"}
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button disabled={busy || !name.trim()} onClick={() => void save()}>
            保存
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label htmlFor="group-name">名称</Label>
          <Input id="group-name" autoFocus placeholder="例如 YouTube、公司内网" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void save()} />
        </div>
        <div className="space-y-1">
          <Label htmlFor="group-note">备注</Label>
          <Input id="group-note" placeholder="选填" value={note} onChange={(e) => setNote(e.target.value)} />
        </div>
      </div>
    </Dialog>
  );
}

/** One entry, edited in place: kind and value save when changed. */
function RuleRow({ rule, onChanged }: { rule: HostRule; onChanged: () => void }) {
  const toast = useToast();
  const [value, setValue] = useState(rule.value);
  useEffect(() => setValue(rule.value), [rule.value]);
  const save = async (kind: RuleKind, v: string) => {
    if (kind === rule.kind && v.trim() === rule.value) return setValue(rule.value);
    try {
      await api.updateRule(rule.id, kind, v);
      onChanged();
    } catch (e) {
      toast(errorText(e), "error");
      setValue(rule.value);
    }
  };
  return (
    <div className="group flex items-center gap-2 px-3 py-1.5">
      <Select aria-label="匹配方式" value={rule.kind} onChange={(e) => void save(e.target.value as RuleKind, value)} className="h-7 w-[118px] shrink-0 bg-transparent text-[12px]">
        {KINDS.map((k) => (
          <option key={k} value={k}>
            {KIND_LABEL[k]}
          </option>
        ))}
      </Select>
      <Input
        aria-label="条目"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => void save(rule.kind, value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") setValue(rule.value);
        }}
        className="h-7 min-w-0 flex-1 bg-transparent font-mono text-[12px]"
      />
      <Button
        variant="ghost"
        size="iconSm"
        aria-label={`删除 ${rule.value}`}
        title="删除"
        className="opacity-0 focus-visible:opacity-100 group-hover:opacity-100"
        onClick={() => void api.deleteRules([rule.id]).then(onChanged, (e) => toast(errorText(e), "error"))}
      >
        <Trash2 />
      </Button>
    </div>
  );
}

function GroupDetail({ group, onChanged, onEdit, onImport }: { group: HostGroup; onChanged: () => void; onEdit: () => void; onImport: () => void }) {
  const toast = useToast();
  const [text, setText] = useState("");
  const [filter, setFilter] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setText("");
    setFilter("");
  }, [group.id]);

  const add = async () => {
    setBusy(true);
    try {
      const r = await api.addRules(group.id, text);
      const parts = [`已添加 ${r.added.rules} 条`, r.added.duplicates ? `${r.added.duplicates} 条已存在` : ""].filter(Boolean);
      if (r.problems.length) {
        toast(`${parts.join("，")}；${r.problems.length} 行无法识别：${r.problems[0].reason}`, "error");
        setText(text.split("\n").filter((_, i) => r.problems.some((p) => p.at === i + 1)).join("\n"));
      } else {
        toast(parts.join("，"));
        setText("");
      }
      onChanged();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    return f ? group.rules.filter((r) => r.value.includes(f)) : group.rules;
  }, [group.rules, filter]);

  return (
    <div className="space-y-5">
      <Group
        label={<span className="normal-case">{group.name}</span>}
        action={
          <div className="flex items-center gap-1">
            <Button size="sm" variant="ghost" onClick={onImport}>
              <Upload />
              导入到此分组
            </Button>
            <Button size="sm" variant="ghost" onClick={onEdit}>
              <Pencil />
              编辑
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(true)}>
              <Trash2 />
              删除
            </Button>
          </div>
        }
      >
        <div className="space-y-2 px-4 py-3">
          {group.note && <p className="text-[12.5px] text-muted">{group.note}</p>}
          <Textarea
            aria-label="添加条目"
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && text.trim()) void add();
            }}
            placeholder={"添加条目，每行一条，例如：\nyoutube.com\n91.108.4.0/22"}
            className="min-h-[76px] font-mono"
          />
          <div className="flex items-center justify-between gap-3">
            <p className="text-[11.5px] leading-[17px] text-muted">不带前缀的域名包括子域名，写 DOMAIN,域名 则仅此域名。⌘/Ctrl + 回车添加。</p>
            <Button size="sm" disabled={busy || !text.trim()} onClick={() => void add()}>
              {busy ? <Loader2 className="animate-spin" /> : <Plus />}
              添加
            </Button>
          </div>
        </div>
      </Group>

      <Group
        label={`条目（${filter ? `${shown.length} / ` : ""}${group.rules.length}）`}
        action={
          group.rules.length > 8 && (
            <div className="relative w-[180px]">
              <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted" />
              <Input aria-label="筛选条目" placeholder="筛选" value={filter} onChange={(e) => setFilter(e.target.value)} className="h-7 pl-7 text-[12px]" />
            </div>
          )
        }
      >
        {group.rules.length === 0 ? (
          <p className="px-4 py-3 text-[12.5px] text-muted">还没有条目，在上面添加或导入。</p>
        ) : (
          shown.map((r) => <RuleRow key={r.id} rule={r} onChanged={onChanged} />)
        )}
      </Group>

      <Dialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        title={`删除分组“${group.name}”？`}
        description={`它的 ${group.rules.length} 条条目会一起删除；统计数据不受影响。`}
        footer={
          <>
            <Button variant="ghost" onClick={() => setConfirmDelete(false)}>
              取消
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                setConfirmDelete(false);
                void api.deleteGroup(group.id).then(onChanged, (e) => toast(errorText(e), "error"));
              }}
            >
              删除
            </Button>
          </>
        }
      />
    </div>
  );
}

export function GroupsPage() {
  const toast = useToast();
  const [groups, setGroups] = useState<HostGroup[] | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [editing, setEditing] = useState<{ open: boolean; group: HostGroup | null }>({ open: false, group: null });
  const [importing, setImporting] = useState<{ open: boolean; preset: number | null }>({ open: false, preset: null });

  const load = useCallback(async () => {
    try {
      const g = await api.hostGroups();
      setGroups(g);
      setSelected((s) => (s !== null && g.some((x) => x.id === s) ? s : (g[0]?.id ?? null)));
    } catch (e) {
      toast(errorText(e), "error");
    }
  }, [toast]);

  useEffect(() => {
    void load();
  }, [load]);

  const exportAll = async () => {
    try {
      if (await api.exportFile("mihomo-monitor-分组.json", await api.exportGroups(), false)) toast("已导出");
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const current = groups?.find((g) => g.id === selected) ?? null;

  return (
    <Page>
      <PageHeader
        title="域名分组"
        subtitle="给域名和 IP 打组，分布页按分组显示流量属于哪个服务。"
        actions={
          <>
            <Button size="sm" variant="outline" onClick={() => setImporting({ open: true, preset: null })}>
              <Upload />
              导入
            </Button>
            <Button size="sm" variant="outline" disabled={!groups?.length} onClick={() => void exportAll()}>
              <Download />
              导出
            </Button>
            <Button size="sm" onClick={() => setEditing({ open: true, group: null })}>
              <FolderPlus />
              新建分组
            </Button>
          </>
        }
      />
      <div className="px-5">
        {groups === null ? (
          <div className="flex items-center gap-2 px-1 text-[12.5px] text-muted">
            <Loader2 className="h-4 w-4 animate-spin" />
            正在读取…
          </div>
        ) : groups.length === 0 ? (
          <div className="rounded-xl border border-dashed border-line px-6 py-10 text-center text-[12.5px] leading-5 text-muted">
            还没有分组。新建一个分组后添加域名或 IP，或者直接导入现成的列表（文本、CSV、JSON、Clash 规则集）。
            <div className="mt-4 flex justify-center gap-2">
              <Button size="sm" variant="outline" onClick={() => setImporting({ open: true, preset: null })}>
                <Upload />
                导入
              </Button>
              <Button size="sm" onClick={() => setEditing({ open: true, group: null })}>
                <FolderPlus />
                新建分组
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="rounded-xl bg-card p-3 shadow-card">
              <div role="tablist" aria-label="分组" className="flex gap-0.5 overflow-x-auto rounded-lg bg-pill p-0.5">
                {groups.map((g) => (
                  <button
                    key={g.id}
                    role="tab"
                    aria-selected={g.id === selected}
                    onClick={() => setSelected(g.id)}
                    className={cn(
                      "flex h-7 min-w-[72px] shrink-0 flex-1 items-center justify-center gap-1.5 whitespace-nowrap rounded-md px-2.5 text-[12.5px] active:!scale-100",
                      g.id === selected ? "bg-card font-medium text-fg shadow-edge dark:bg-white/[.12]" : "text-muted hover:text-fg",
                    )}
                  >
                    {g.name}
                    <span className="font-mono text-[10.5px] tabular-nums text-faint">{g.rules.length}</span>
                  </button>
                ))}
              </div>
            </div>
            {current && (
              <div key={current.id} className="view-in">
                <GroupDetail
                  group={current}
                  onChanged={() => void load()}
                  onEdit={() => setEditing({ open: true, group: current })}
                  onImport={() => setImporting({ open: true, preset: current.id })}
                />
              </div>
            )}
          </div>
        )}
        <p className="mt-5 px-1 text-[11.5px] leading-relaxed text-muted">
          条目的匹配方式与 Clash 规则相同：「仅此域名」只匹配这个域名，「域名及子域名」还包括它的所有子域名，「包含关键字」匹配域名里含有这段文字的，「IP / 网段」匹配地址。
          一个域名同时符合多条时，依次按仅此域名、最长的子域名规则、关键字判断；同一条目出现在多个分组时，先建的分组优先。
        </p>
      </div>
      <GroupDialog
        open={editing.open}
        group={editing.group}
        onClose={() => setEditing({ open: false, group: null })}
        onSaved={(id) => {
          setSelected(id);
          void load();
        }}
      />
      <ImportDialog open={importing.open} groups={groups ?? []} preset={importing.preset} onClose={() => setImporting({ open: false, preset: null })} onDone={() => void load()} />
    </Page>
  );
}
