import { Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Segmented } from "@/components/Segmented";
import { Button } from "@/components/ui/button";
import { Dialog } from "@/components/ui/dialog";
import { Input, Label, Select } from "@/components/ui/input";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type HostGroup, type RuleKind } from "@/lib/api";
import { asLine, isAddress } from "@/lib/hostgroup";

const NEW = "__new__";

/** Puts a host seen in the traffic into a group, new or existing. */
export function AddToGroupDialog({ host, onClose, onDone }: { host: string | null; onClose: () => void; onDone: () => void }) {
  const toast = useToast();
  const [groups, setGroups] = useState<HostGroup[]>([]);
  const [target, setTarget] = useState<string>(NEW);
  const [newName, setNewName] = useState("");
  const [kind, setKind] = useState<RuleKind>("suffix");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (host === null) return;
    setValue(host);
    setKind(isAddress(host) ? "cidr" : "suffix");
    setNewName("");
    api.hostGroups().then(
      (g) => {
        setGroups(g);
        setTarget(g.length ? String(g[0].id) : NEW);
      },
      () => setGroups([]),
    );
  }, [host]);

  if (host === null) return null;
  const address = isAddress(host);

  const save = async () => {
    setBusy(true);
    try {
      const id = target === NEW ? await api.createGroup(newName, "") : Number(target);
      const r = await api.addRules(id, asLine(kind, value.trim()));
      if (r.problems.length) throw new Error(r.problems[0].reason);
      toast(r.added.rules ? "已加入分组" : "这个分组里已经有这一条");
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
      title="加入分组"
      description="之后在分布页按分组查看时，这类流量会合并显示为分组的名称。"
      footer={
        <>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          <Button disabled={busy || !value.trim() || (target === NEW && !newName.trim())} onClick={() => void save()}>
            {busy && <Loader2 className="animate-spin" />}
            加入
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <div className="space-y-1">
          <Label>分组</Label>
          <div className="flex gap-2">
            <Select aria-label="分组" value={target} onChange={(e) => setTarget(e.target.value)} className="w-[180px]">
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
        <div className="space-y-1.5">
          <Label htmlFor="add-value">条目</Label>
          {!address && (
            <Segmented
              label="匹配方式"
              value={kind}
              onChange={setKind}
              options={[
                ["suffix", "包括子域名"],
                ["domain", "仅此域名"],
              ]}
            />
          )}
          <Input id="add-value" value={value} onChange={(e) => setValue(e.target.value)} />
          <p className="text-[11.5px] leading-[17px] text-muted">
            {address
              ? "可以改成网段，例如 91.108.4.0/22，把同一服务的一段地址一起归入。"
              : "可以改成上一级域名，例如把 rr3---sn.googlevideo.com 改成 googlevideo.com，同一服务的子域名会一起归入。"}
          </p>
        </div>
      </div>
    </Dialog>
  );
}
