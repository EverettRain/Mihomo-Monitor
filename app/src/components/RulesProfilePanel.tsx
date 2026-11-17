import { ChevronDown, FileCode2, FileUp, FolderOpen, Loader2, Plus, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Segmented } from "@/components/Segmented";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Input, Label } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type RulesPlan, type RulesProfile, type RulesProfileSummary } from "@/lib/api";
import { cn } from "@/lib/utils";

const isMac = navigator.userAgent.includes("Mac");
const REVEAL = isMac ? "在访达中显示" : "在资源管理器中显示";

function ProfileIcon({ className }: { className?: string }) {
  return (
    <span className={cn("flex shrink-0 items-center justify-center rounded-[9px] bg-pill text-fg-2", className)}>
      <FileCode2 className="h-4 w-4" />
    </span>
  );
}

const counts = (groups: number, rules: number) => `${groups} 个分组 · ${rules} 条归属规则`;

/** A toast line for what reading an edited configuration file changed; empty when nothing did. */
export function describeChanges(plan: RulesPlan | null): string {
  if (!plan) return "";
  const part = (title: string, p: RulesPlan["groups"]) => {
    if (!p) return "";
    const bits = [p.added.length && `新增 ${p.added.length}`, p.changed.length && `修改 ${p.changed.length}`, p.removed.length && `删除 ${p.removed.length}`].filter(Boolean);
    return bits.length ? `${title}${bits.join("、")}` : "";
  };
  const parts = [part("分组", plan.groups), part("归属规则", plan.attribution)].filter(Boolean);
  if (!parts.length) return "";
  const skipped = plan.problems.length ? `；${plan.problems.length} 处无法识别，已跳过` : "";
  return `已从配置文件重新读取：${parts.join("；")}${skipped}`;
}

/** Names a new configuration: empty, or a copy of the one in use. */
function CreateDialog({ open, onClose, onCreated }: { open: boolean; onClose: () => void; onCreated: (name: string) => void }) {
  const toast = useToast();
  const [name, setName] = useState("");
  const [copy, setCopy] = useState<"empty" | "copy">("copy");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (open) {
      setName("");
      setCopy("copy");
    }
  }, [open]);
  if (!open) return null;
  const create = async () => {
    setBusy(true);
    try {
      onCreated(await api.createRulesProfile(name, copy === "copy"));
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
      title="新建配置"
      description="新配置是数据目录 rules 文件夹里的一个 YAML 文件，建好后切换过去。"
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button disabled={busy || !name.trim()} onClick={() => void create()}>
            {busy && <Loader2 className="animate-spin" />}
            新建并切换
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label htmlFor="profile-name">名称</Label>
          <Input id="profile-name" autoFocus placeholder="例如 工作、家里" value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void create()} />
        </div>
        <div className="space-y-1.5">
          <Label>内容</Label>
          <Segmented
            label="新配置的内容"
            value={copy}
            onChange={setCopy}
            options={[
              ["copy", "复制当前配置"],
              ["empty", "空白"],
            ]}
          />
        </div>
      </div>
    </Dialog>
  );
}

/** Every configuration as a card to switch to, with ways to add one. */
function Picker({ onPick, onCancel, onAdded }: { onPick: (name: string) => void; onCancel: () => void; onAdded: (name: string) => void }) {
  const toast = useToast();
  const file = useRef<HTMLInputElement>(null);
  const [list, setList] = useState<RulesProfileSummary[] | null>(null);
  const [creating, setCreating] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);

  const load = () =>
    api.rulesProfiles().then(setList, (e) => {
      setList([]);
      toast(errorText(e), "error");
    });
  useEffect(() => {
    void load();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !document.querySelector("[data-dialog-layer]")) onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const importFile = (f: File | undefined) => {
    if (!f) return;
    if (f.size > 20 * 1024 * 1024) return toast("文件太大（超过 20 MB）", "error");
    f.text().then(
      (text) =>
        api.importRulesProfile(f.name.replace(/\.(ya?ml|json)$/i, ""), text).then(
          (name) => onAdded(name),
          (e) => toast(errorText(e), "error"),
        ),
      () => toast("无法读取这个文件", "error"),
    );
  };

  const remove = async (name: string) => {
    try {
      await api.deleteRulesProfile(name);
      setDeleting(null);
      void load();
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <div className="space-y-3 p-4">
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="text-[14px] font-semibold">选择配置</h2>
          <p className="text-[12.5px] text-muted">每个配置是一个 YAML 文件，包含域名分组和归属规则。用其他编辑器改了文件，回到这里会自动重新读取。</p>
        </div>
        <Button size="iconSm" variant="ghost" aria-label="收起" title="收起（Esc）" onClick={onCancel}>
          <X />
        </Button>
      </div>
      {list === null ? (
        <div className="flex items-center gap-2 py-3 text-[12.5px] text-muted">
          <Loader2 className="h-4 w-4 animate-spin" />
          正在读取…
        </div>
      ) : (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-2">
          {list.map((p) => (
            <div key={p.name} className="group relative">
              <button
                onClick={() => onPick(p.name)}
                aria-current={p.active || undefined}
                className={cn(
                  "flex w-full min-w-0 items-center gap-2.5 rounded-xl bg-card-2 px-3 py-2.5 text-left hover:bg-pill",
                  p.active && "shadow-[inset_0_0_0_1.5px_rgb(var(--accent))]",
                )}
              >
                <ProfileIcon className="h-8 w-8" />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5">
                    <span className="truncate text-[13px] font-medium">{p.name}</span>
                    {p.active && <Tag>使用中</Tag>}
                  </span>
                  <span className={cn("block truncate text-[12px]", p.error ? "text-warn" : "text-muted")}>{p.error ? "文件有错误" : counts(p.groups, p.rules)}</span>
                </span>
              </button>
              {!p.active && (
                <Button
                  size="iconSm"
                  variant="ghost"
                  aria-label={`删除配置 ${p.name}`}
                  title="删除"
                  className="absolute right-1.5 top-1/2 -translate-y-1/2 opacity-0 focus-visible:opacity-100 group-hover:opacity-100"
                  onClick={() => setDeleting(p.name)}
                >
                  <Trash2 />
                </Button>
              )}
            </div>
          ))}
        </div>
      )}
      <div className="flex items-center gap-2">
        <Button size="sm" variant="outline" onClick={() => setCreating(true)}>
          <Plus />
          新建配置
        </Button>
        <Button size="sm" variant="outline" onClick={() => file.current?.click()}>
          <FileUp />
          导入文件
        </Button>
        <input ref={file} type="file" accept=".yaml,.yml,.json" className="hidden" onChange={(e) => importFile(e.target.files?.[0])} />
      </div>
      <CreateDialog open={creating} onClose={() => setCreating(false)} onCreated={onAdded} />
      <Dialog
        open={deleting !== null}
        onClose={() => setDeleting(null)}
        title="删除配置"
        description={deleting ? `「${deleting}」的文件会被删除，其中的分组和归属规则无法恢复；统计数据不受影响。` : undefined}
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
    </div>
  );
}

/**
 * The rule configuration in use, above the host groups: its file can be opened or shown, and
 * another one switched to. `onChanged` runs after a switch, to read the groups again.
 */
export function RulesProfilePanel({ profile, onChanged }: { profile: RulesProfile | null; onChanged: () => void }) {
  const toast = useToast();
  const [picking, setPicking] = useState(false);

  const run = (f: () => Promise<void>) => f().catch((e) => toast(errorText(e), "error"));
  const pick = (name: string) =>
    void run(async () => {
      if (name !== profile?.name) {
        await api.switchRulesProfile(name);
        toast(`已切换到配置「${name}」`);
        onChanged();
      }
      setPicking(false);
    });

  return (
    <div className="mb-5 overflow-hidden rounded-xl bg-card shadow-card">
      {picking ? (
        <Picker onPick={pick} onCancel={() => setPicking(false)} onAdded={pick} />
      ) : (
        <>
          <div className="flex items-center gap-2.5 px-3 py-3">
            <ProfileIcon className="h-8 w-8" />
            <button className="min-w-0 flex-1 text-left" onClick={() => setPicking(true)} title="切换配置">
              <span className="flex items-center gap-1.5">
                <span className="truncate text-[13.5px] font-semibold">{profile?.name ?? "…"}</span>
                {profile && <Tag>{counts(profile.groups, profile.rules)}</Tag>}
              </span>
              <span className="block truncate font-mono text-[11.5px] text-muted" title={profile?.path}>
                {profile?.path ?? ""}
              </span>
            </button>
            <Button size="iconSm" variant="ghost" aria-label={REVEAL} title={REVEAL} disabled={!profile} onClick={() => void run(api.revealRulesProfile)}>
              <FolderOpen />
            </Button>
            <Button size="sm" variant="outline" disabled={!profile} onClick={() => void run(api.openRulesProfile)}>
              打开文件
            </Button>
            <Button size="sm" variant="outline" onClick={() => setPicking(true)}>
              切换配置
              <ChevronDown />
            </Button>
          </div>
          {profile?.error && (
            <p className="selectable mx-3 mb-3 rounded-lg bg-warn-soft px-3 py-2 text-[12px] leading-[18px] text-warn">
              配置文件有错误，暂时沿用上次读到的分组和规则：{profile.error}
            </p>
          )}
        </>
      )}
    </div>
  );
}
