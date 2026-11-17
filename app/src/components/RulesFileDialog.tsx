import { Download, FileUp, Loader2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Segmented } from "@/components/Segmented";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type PartPlan, type RulesPlan } from "@/lib/api";
import { cn } from "@/lib/utils";

type Mode = "replace" | "merge";

const MODES: readonly (readonly [Mode, string])[] = [
  ["replace", "以文件为准"],
  ["merge", "只新增和更新"],
];

/** A few names, then how many more. */
function names(list: string[]): string {
  return list.length > 4 ? `${list.slice(0, 4).join("、")} 等 ${list.length} 个` : list.join("、");
}

function Part({ title, plan }: { title: string; plan: PartPlan | null }) {
  if (!plan) return <div className="text-muted">{title}：文件里没有这一部分，保持不变</div>;
  const parts: [string, string[], string][] = [
    ["新增", plan.added, "text-fg"],
    ["修改", plan.changed, "text-fg"],
    ["删除", plan.removed, "text-danger"],
  ];
  const shown = parts.filter(([, list]) => list.length > 0);
  return (
    <div>
      <span className="text-fg-2">{title}：</span>
      {shown.length === 0 && <span className="text-muted">没有变化</span>}
      {shown.map(([verb, list, tone], i) => (
        <span key={verb} className={tone}>
          {i > 0 && "；"}
          {verb} {list.length}（{names(list)}）
        </span>
      ))}
      {plan.unchanged > 0 && <span className="text-muted">；{plan.unchanged} 项不变</span>}
    </div>
  );
}

const changes = (p: PartPlan | null) => (p ? p.added.length + p.changed.length + p.removed.length : 0);

/**
 * The host groups and attribution rules as one YAML file: export it, change many groups at once
 * in any editor, import it back. What an import would change shows before anything is saved.
 */
export function RulesFileDialog({ open, onClose, onDone }: { open: boolean; onClose: () => void; onDone: () => void }) {
  const toast = useToast();
  const file = useRef<HTMLInputElement>(null);
  const [text, setText] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("replace");
  const [plan, setPlan] = useState<RulesPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setText("");
    setFileName(null);
    setMode("replace");
    setPlan(null);
    setError(null);
  }, [open]);

  // Read what the import would do a moment after the text or the mode changes.
  useEffect(() => {
    if (!open || !text.trim()) {
      setPlan(null);
      setError(null);
      return;
    }
    const timer = window.setTimeout(() => {
      api.planRulesFile(text, mode === "merge").then(
        (p) => {
          setPlan(p);
          setError(null);
        },
        (e) => {
          setPlan(null);
          setError(errorText(e));
        },
      );
    }, 250);
    return () => window.clearTimeout(timer);
  }, [open, text, mode]);

  if (!open) return null;

  const exportNow = async () => {
    try {
      if (await api.exportFile("mihomo-monitor-规则.yaml", await api.exportRulesFile(), "yaml")) toast("已导出");
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const readFile = (f: File | undefined) => {
    if (!f) return;
    if (f.size > 20 * 1024 * 1024) {
      toast("文件太大（超过 20 MB）", "error");
      return;
    }
    f.text().then(
      (t) => {
        setText(t);
        setFileName(f.name);
      },
      () => toast("无法读取这个文件", "error"),
    );
  };

  const apply = async () => {
    setBusy(true);
    try {
      const done = await api.applyRulesFile(text, mode === "merge");
      const n = changes(done.groups) + changes(done.attribution);
      toast(n ? `已导入，${n} 处变化` : "已导入，没有变化");
      onDone();
      onClose();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const total = plan ? changes(plan.groups) + changes(plan.attribution) : 0;
  const removed = plan ? (plan.groups?.removed.length ?? 0) + (plan.attribution?.removed.length ?? 0) : 0;

  return (
    <Dialog
      open
      onClose={() => !busy && onClose()}
      title="规则配置文件"
      description="域名分组与归属规则保存在同一个 YAML 文件里：导出后可以用任意编辑器一次修改多个分组，再导入。条目写法与 Clash 规则相同。"
      className="max-w-[600px]"
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button variant={removed > 0 ? "destructive" : "default"} disabled={busy || !plan || total === 0} onClick={() => void apply()}>
            {busy && <Loader2 className="animate-spin" />}
            {removed > 0 ? `导入并删除 ${removed} 项` : "导入"}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="flex items-center justify-between gap-3 rounded-xl bg-card-2 px-3 py-2.5 shadow-edge">
          <span className="text-[12.5px] text-fg-2">把当前的全部分组和归属规则保存为文件</span>
          <Button size="sm" variant="outline" onClick={() => void exportNow()}>
            <Download />
            导出
          </Button>
        </div>

        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={() => file.current?.click()}>
            <FileUp />
            选择文件
          </Button>
          <input ref={file} type="file" accept=".yaml,.yml,.json" className="hidden" onChange={(e) => readFile(e.target.files?.[0])} />
          <span className="min-w-0 flex-1 truncate text-[12px] text-muted">{fileName ?? "或把文件内容粘贴到下面"}</span>
          <Segmented label="导入方式" value={mode} onChange={setMode} options={MODES} />
        </div>
        <Textarea
          aria-label="规则配置文件的内容"
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setFileName(null);
          }}
          placeholder={"groups:\n  - name: YouTube\n    entries:\n      - DOMAIN-SUFFIX,youtube.com\nattribution:\n  - name: \"Container: us-workstation\"\n    inbound: US-Workstation-Port"}
          className="min-h-[140px] font-mono text-[12px]"
        />
        <p className="text-[11.5px] leading-[17px] text-muted">
          {mode === "replace"
            ? "以文件为准：文件里没有的分组和归属规则会被删除，分组的优先顺序按文件里的先后。文件里整段没有 groups 或 attribution 时，那部分保持不变。"
            : "只新增和更新：同名分组的备注和条目换成文件里的，同一入站的归属规则按文件更新，其余新增；文件里没有的保持不变。"}
        </p>

        {error && <p className="selectable text-[12px] text-danger">{error}</p>}
        {plan && (
          <div className="space-y-1.5 rounded-xl bg-card-2 px-3 py-2.5 text-[12px] leading-[18px] shadow-edge">
            <Part title="域名分组" plan={plan.groups} />
            <Part title="归属规则" plan={plan.attribution} />
            {plan.problems.length > 0 && (
              <div className="max-h-[96px] overflow-y-auto text-[11.5px] text-warn">
                {plan.problems.slice(0, 30).map((p, i) => (
                  <div key={i} className={cn("selectable truncate")}>
                    {p.text}：{p.reason}
                  </div>
                ))}
                {plan.problems.length > 30 && <div>… 还有 {plan.problems.length - 30} 处</div>}
              </div>
            )}
          </div>
        )}
      </div>
    </Dialog>
  );
}
