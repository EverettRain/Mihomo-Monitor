import { FileUp, Loader2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Input, Label, Select, Textarea } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type HostGroup, type ImportFormat, type ImportPreview } from "@/lib/api";
import { FORMAT_LABEL, KIND_LABEL } from "@/lib/hostgroup";

const NEW = "__new__";
const FORMATS: ImportFormat[] = ["auto", "text", "csv", "json", "yaml"];
const ACCEPT = ".txt,.list,.conf,.csv,.tsv,.json,.yaml,.yml";

/**
 * Imports entries from pasted text or a file. Shows what was read before anything is saved;
 * entries that name no group go to the group chosen here.
 */
export function ImportDialog({
  open,
  groups,
  preset,
  onClose,
  onDone,
}: {
  open: boolean;
  groups: HostGroup[];
  /** Group to import into by default. */
  preset: number | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const toast = useToast();
  const file = useRef<HTMLInputElement>(null);
  const [text, setText] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [format, setFormat] = useState<ImportFormat>("auto");
  const [target, setTarget] = useState<string>(NEW);
  const [newName, setNewName] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setText("");
    setFileName(null);
    setFormat("auto");
    setPreview(null);
    setNewName("");
    setTarget(preset !== null ? String(preset) : groups.length ? String(groups[0].id) : NEW);
  }, [open, preset, groups]);

  // Read as the user types, a moment after they stop.
  useEffect(() => {
    if (!open || !text.trim()) {
      setPreview(null);
      return;
    }
    const timer = window.setTimeout(() => {
      api.importPreview(text, format).then(setPreview, (e) => toast(errorText(e), "error"));
    }, 250);
    return () => window.clearTimeout(timer);
  }, [open, text, format, toast]);

  if (!open) return null;

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
        const ext = f.name.split(".").pop()?.toLowerCase();
        setFormat(ext === "csv" || ext === "tsv" ? "csv" : ext === "json" ? "json" : ext === "yaml" || ext === "yml" ? "yaml" : "auto");
      },
      () => toast("无法读取这个文件", "error"),
    );
  };

  const ungrouped = preview?.entries.filter((e) => !e.group).length ?? 0;
  const named = new Set(preview?.entries.map((e) => e.group).filter(Boolean));
  const targetName = target === NEW ? newName.trim() : (groups.find((g) => String(g.id) === target)?.name ?? "");
  const ready = !!preview?.entries.length && (ungrouped === 0 || targetName !== "");

  const apply = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      const r = await api.importApply(preview.entries, targetName);
      toast(
        [`已导入 ${r.rules} 条`, r.groups ? `新建 ${r.groups} 个分组` : "", r.duplicates ? `${r.duplicates} 条已存在，跳过` : ""].filter(Boolean).join("，"),
      );
      onDone();
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
      title="导入域名与 IP"
      description="粘贴文本或选择文件。支持每行一条的列表（域名、IP、网段、网址、Clash 规则行、+.example.com）、CSV、JSON（含 sing-box rule-set）和 Clash rule-provider 的 YAML。"
      className="max-w-[600px]"
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button disabled={busy || !ready} onClick={() => void apply()}>
            {busy && <Loader2 className="animate-spin" />}
            导入 {preview?.entries.length ? `${preview.entries.length} 条` : ""}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={() => file.current?.click()}>
            <FileUp />
            选择文件
          </Button>
          <input ref={file} type="file" accept={ACCEPT} className="hidden" onChange={(e) => readFile(e.target.files?.[0])} />
          <span className="min-w-0 flex-1 truncate text-[12px] text-muted">{fileName ?? "或直接粘贴到下面"}</span>
          <Select aria-label="格式" value={format} onChange={(e) => setFormat(e.target.value as ImportFormat)} className="h-7 w-auto text-[12.5px]">
            {FORMATS.map((f) => (
              <option key={f} value={f}>
                {FORMAT_LABEL[f]}
              </option>
            ))}
          </Select>
        </div>
        <Textarea
          aria-label="要导入的内容"
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setFileName(null);
          }}
          placeholder={"youtube.com\n+.googlevideo.com\nDOMAIN-SUFFIX,ytimg.com\n91.108.4.0/22"}
          className="min-h-[140px]"
        />

        {preview && (
          <div className="space-y-2 rounded-xl bg-card-2 px-3 py-2.5 text-[12px] shadow-edge">
            <div className="text-fg-2">
              识别为 {FORMAT_LABEL[preview.format]} · <span className="font-medium text-fg">{preview.entries.length}</span> 条
              {named.size > 0 && ` · 其中 ${preview.entries.length - ungrouped} 条自带分组（${[...named].slice(0, 3).join("、")}${named.size > 3 ? " 等" : ""}）`}
              {preview.problems.length > 0 && <span className="text-warn"> · {preview.problems.length} 处无法识别</span>}
            </div>
            {preview.entries.length > 0 && (
              <div className="max-h-[96px] overflow-y-auto font-mono text-[11.5px] leading-[19px] text-fg-2">
                {preview.entries.slice(0, 50).map((e, i) => (
                  <div key={i} className="flex gap-2">
                    <span className="w-[84px] shrink-0 font-sans text-muted">{KIND_LABEL[e.kind]}</span>
                    <span className="selectable truncate">{e.value}</span>
                    {e.group && <span className="ml-auto shrink-0 font-sans text-muted">{e.group}</span>}
                  </div>
                ))}
                {preview.entries.length > 50 && <div className="font-sans text-muted">… 还有 {preview.entries.length - 50} 条</div>}
              </div>
            )}
            {preview.problems.length > 0 && (
              <div className="max-h-[72px] overflow-y-auto text-[11.5px] leading-[18px] text-warn">
                {preview.problems.slice(0, 20).map((p, i) => (
                  <div key={i} className="selectable truncate">
                    {p.at > 0 ? `第 ${p.at} ${preview.format === "json" ? "项" : "行"}：` : ""}
                    {p.reason}
                    {p.text && <span className="text-muted">（{p.text}）</span>}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {(ungrouped > 0 || !preview) && (
          <div className="space-y-1">
            <Label>{named.size > 0 ? "没写分组的条目加入" : "加入分组"}</Label>
            <div className="flex gap-2">
              <Select aria-label="目标分组" value={target} onChange={(e) => setTarget(e.target.value)} className="w-[200px]">
                {groups.map((g) => (
                  <option key={g.id} value={g.id}>
                    {g.name}
                  </option>
                ))}
                <option value={NEW}>新建分组…</option>
              </Select>
              {target === NEW && <Input aria-label="新分组名称" placeholder="例如 YouTube" value={newName} onChange={(e) => setNewName(e.target.value)} />}
            </div>
          </div>
        )}
      </div>
    </Dialog>
  );
}
