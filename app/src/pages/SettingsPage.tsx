import { listen } from "@tauri-apps/api/event";
import { FolderOpen } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Group, Page, Row } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type AppInfo, type Settings } from "@/lib/api";

export function SettingsPage({ info }: { info: AppInfo | null }) {
  const toast = useToast();
  const [settings, setSettings] = useState<Settings | null>(null);

  const reload = useCallback(async () => {
    try {
      setSettings(await api.getSettings());
    } catch (e) {
      toast(errorText(e), "error");
    }
  }, [toast]);

  useEffect(() => {
    void reload();
    // The tray menu can switch lightweight mode too.
    const un = listen("settings-changed", () => void reload());
    return () => void un.then((f) => f());
  }, [reload]);

  const change = async (apply: () => Promise<void>, patch: Partial<Settings>) => {
    if (!settings) return;
    const before = settings;
    setSettings({ ...settings, ...patch });
    try {
      await apply();
    } catch (e) {
      setSettings(before);
      toast(errorText(e), "error");
    }
  };

  return (
    <Page>
      <h1 className="sr-only">设置</h1>
      <div className="space-y-5 px-5 pt-3">
        <Group label="运行">
          <Row label="开机自启" hint="登录后在托盘中启动并开始统计，不弹出窗口。统计只在 Mihomo-Monitor 运行时进行。">
            <Switch
              label="开机自启"
              checked={settings?.autostart ?? false}
              disabled={!settings}
              onCheckedChange={(v) => void change(() => api.setAutostart(v), { autostart: v })}
            />
          </Row>
          <Row label="轻量模式" hint="关闭窗口时释放界面占用的内存，只留后台统计；再次打开窗口需要约 1 秒。关闭后，关窗只是隐藏，再打开更快。">
            <Switch
              label="轻量模式"
              checked={settings?.lightweight ?? true}
              disabled={!settings}
              onCheckedChange={(v) => void change(() => api.setLightweight(v), { lightweight: v })}
            />
          </Row>
        </Group>

        <Group label="统计">
          <Row label="每周从周日开始" hint="影响概览中的“本周”。默认从周一开始。">
            <Switch
              label="每周从周日开始"
              checked={settings?.weekStartsSunday ?? false}
              disabled={!settings}
              onCheckedChange={(v) => void change(() => api.setWeekStartsSunday(v), { weekStartsSunday: v })}
            />
          </Row>
        </Group>

        <Group label="数据">
          <Row label="数据目录" hint={<span className="selectable break-all font-mono">{info?.dataDir ?? "…"}</span>}>
            <Button size="sm" variant="outline" onClick={() => void api.openDataDir().catch((e) => toast(errorText(e), "error"))}>
              <FolderOpen />
              打开
            </Button>
          </Row>
        </Group>

        <Group>
          <Row label="退出 Mihomo-Monitor" hint="关闭窗口不会退出；退出后停止统计，直到下次打开。">
            <Button size="sm" variant="outline" onClick={() => void api.quitApp()}>
              退出
            </Button>
          </Row>
        </Group>
      </div>
    </Page>
  );
}
