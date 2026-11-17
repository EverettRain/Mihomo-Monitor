import { listen } from "@tauri-apps/api/event";
import { BellRing, FolderOpen } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Group, Page, Row } from "@/components/PageHeader";
import { Segmented } from "@/components/Segmented";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { useToast } from "@/components/ui/toast";
import { api, errorText, type AppInfo, type Settings } from "@/lib/api";

const SAMPLE_CHOICES: readonly (readonly [number, string])[] = [
  [500, "0.5 秒"],
  [1000, "1 秒"],
  [2000, "2 秒"],
];

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
          <Row
            label="采样间隔"
            hint="多久读一次各内核的连接列表。间隔越短，两次读取之间开了又关、看不出去向的连接（漏采）越少；每次读取约占 1 毫秒 / 千条连接。修改后立即生效。"
          >
            <Segmented
              label="采样间隔"
              value={settings?.sampleMs ?? 1000}
              onChange={(ms) => void change(() => api.setSampleInterval(ms), { sampleMs: ms })}
              options={SAMPLE_CHOICES}
            />
          </Row>
        </Group>

        <Group label="通知">
          <Row
            label="测试通知"
            hint="立即发一条通知，确认配额提醒能弹出来。macOS 第一次发通知时会询问是否允许；之后可在「系统设置 → 通知」里修改。"
          >
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                void api.testNotification().then(
                  (r) =>
                    toast(
                      r.dev && navigator.userAgent.includes("Mac")
                        ? "已发送。开发版在 macOS 上以「终端」的名义发通知，没看到的话，请在「系统设置 → 通知 → 终端」里允许通知。"
                        : "已发送。没看到的话，请检查系统设置里 Mihomo-Monitor 的通知权限和勿扰模式。",
                    ),
                  (e) => toast(errorText(e), "error"),
                )
              }
            >
              <BellRing />
              发送测试通知
            </Button>
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
