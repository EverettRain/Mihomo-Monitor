import { Loader2, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Group, Page, PageHeader, Row } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type Source } from "@/lib/api";
import { cn } from "@/lib/utils";

const STATUS: Record<Source["status"], { dot: string; text: string }> = {
  online: { dot: "bg-ok shadow-[0_0_0_3px_rgb(var(--ok-soft))]", text: "正在统计" },
  connecting: { dot: "bg-warn", text: "正在连接" },
  offline: { dot: "bg-faint", text: "未连接" },
};

function SourceRow({ s, onRemove }: { s: Source; onRemove?: () => void }) {
  const st = STATUS[s.status];
  return (
    <div className="flex items-start gap-3 px-4 py-3">
      <span className={cn("mt-[6px] h-[7px] w-[7px] shrink-0 rounded-full", st.dot)} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          <span className="text-[13px] font-medium">{s.name}</span>
          <span className="text-[12px] text-muted">{st.text}</span>
          {s.version && <span className="font-mono text-[11px] text-muted">{s.version}</span>}
          {s.manual && <span className="rounded bg-pill px-1.5 text-[10.5px] text-muted">手动添加</span>}
        </div>
        {s.endpoint && <div className="selectable mt-0.5 truncate font-mono text-[11.5px] text-fg-2">{s.endpoint}</div>}
        {s.message && s.status !== "online" && <div className="mt-0.5 text-[12px] leading-[18px] text-muted">{s.message}</div>}
      </div>
      {onRemove && (
        <Button size="iconSm" variant="ghost" aria-label={`移除 ${s.name}`} title="移除" onClick={onRemove}>
          <Trash2 />
        </Button>
      )}
    </div>
  );
}

export function SourcesPage({ sources }: { sources: Source[] | null }) {
  const toast = useToast();
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [auto, setAuto] = useState<boolean | null>(null);

  useEffect(() => {
    void api.getSettings().then((s) => setAuto(s.autoDiscover)).catch(() => {});
  }, []);

  const toggleAuto = async (v: boolean) => {
    setAuto(v);
    try {
      await api.setAutoDiscover(v);
    } catch (e) {
      setAuto(!v);
      toast(errorText(e), "error");
    }
  };

  const add = async () => {
    setBusy(true);
    try {
      await api.addManualSource(name, url, secret);
      setName("");
      setUrl("");
      setSecret("");
      toast("已添加，正在连接…");
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const remove = async (s: Source) => {
    const target = s.key.replace(/^manual:/, "");
    try {
      await api.removeManualSource(target);
      toast(`已移除 ${s.name}`);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <Page>
      <PageHeader title="内核" subtitle="自动发现本机正在运行的 mihomo 内核，读取它的流量计数；不会改动客户端的任何设置。" />
      <div className="space-y-5 px-5">
        <Group>
          <Row label="自动发现本机内核" hint="关闭后只读取手动添加的内核，例如只想统计路由器的流量时。">
            <Switch label="自动发现本机内核" checked={auto ?? true} disabled={auto === null} onCheckedChange={(v) => void toggleAuto(v)} />
          </Row>
        </Group>

        <Group label="内核">
          {sources === null ? (
            <div className="flex items-center gap-2 px-4 py-3 text-[12.5px] text-muted">
              <Loader2 className="h-4 w-4 animate-spin" />
              正在查找…
            </div>
          ) : sources.length === 0 ? (
            <div className="px-4 py-3 text-[12.5px] leading-5 text-muted">没有发现正在运行的 mihomo 内核。打开 Clash Verge Rev 等客户端后会自动出现在这里。</div>
          ) : (
            sources.map((s) => <SourceRow key={s.key} s={s} onRemove={s.manual ? () => void remove(s) : undefined} />)
          )}
        </Group>

        <Group label="手动添加">
          <div className="space-y-3 px-4 py-3">
            <p className="text-[12px] leading-[18px] text-muted">
              用于路由器上的内核（OpenClash 等），或没有被自动发现的内核。填写配置里的 external-controller 地址和 secret。
            </p>
            <div className="grid grid-cols-[1fr_1.4fr_1fr] gap-2">
              <div className="space-y-1">
                <Label htmlFor="src-name">名称</Label>
                <Input id="src-name" placeholder="例如 路由器" value={name} onChange={(e) => setName(e.target.value)} />
              </div>
              <div className="space-y-1">
                <Label htmlFor="src-url">控制地址</Label>
                <Input id="src-url" placeholder="192.168.1.1:9090" value={url} onChange={(e) => setUrl(e.target.value)} />
              </div>
              <div className="space-y-1">
                <Label htmlFor="src-secret">secret</Label>
                <Input id="src-secret" type="password" placeholder="没有可留空" value={secret} onChange={(e) => setSecret(e.target.value)} />
              </div>
            </div>
            <div className="flex justify-end">
              <Button disabled={busy || !url.trim()} onClick={() => void add()}>
                {busy ? <Loader2 className="animate-spin" /> : <Plus />}
                添加
              </Button>
            </div>
          </div>
        </Group>

        <p className="px-1 text-[11.5px] leading-[18px] text-muted">
          总量取自内核自身的累计计数，Mihomo-Monitor 退出或电脑睡眠期间的流量，只要内核没有重启，下次读取时会补回。内核和 Mihomo-Monitor
          同时停止期间的流量无法找回。
        </p>
      </div>
    </Page>
  );
}
