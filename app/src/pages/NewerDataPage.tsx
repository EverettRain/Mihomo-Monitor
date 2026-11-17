import { check, type Update } from "@tauri-apps/plugin-updater";
import { ArrowDownToLine, Check, DatabaseZap, FolderOpen, Loader2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import appIcon from "@/assets/app-icon.png";
import { WindowControls } from "@/components/AppShell";
import { ReleaseNotes } from "@/components/ReleaseNotes";
import { Button } from "@/components/ui/button";
import { api, errorText, type NewerData } from "@/lib/api";
import { cn } from "@/lib/utils";

const isMac = navigator.userAgent.includes("Mac");
const mb = (n: number) => (n / 1048576).toFixed(1);

type State =
  | { kind: "checking" }
  | { kind: "none" }
  | { kind: "failed"; message: string }
  | { kind: "found"; update: Update }
  | { kind: "downloading"; update: Update; done: number; total: number }
  | { kind: "ready"; update: Update }
  | { kind: "install-failed"; update: Update; message: string };

/**
 * Shown instead of the app when the statistics on disk were written by a later version (a
 * development build, typically). This build leaves them alone and only offers to update itself,
 * so an older install can still be brought up to date instead of failing to start.
 */
export function NewerDataPage({ data }: { data: NewerData }) {
  const [state, setState] = useState<State>({ kind: "checking" });

  const look = useCallback(async () => {
    setState({ kind: "checking" });
    try {
      const update = await check({ timeout: 15000 });
      setState(update ? { kind: "found", update } : { kind: "none" });
    } catch (e) {
      setState({ kind: "failed", message: errorText(e) });
    }
  }, []);

  useEffect(() => {
    void look();
  }, [look]);

  const install = async (update: Update) => {
    setState({ kind: "downloading", update, done: 0, total: 0 });
    try {
      let done = 0;
      let total = 0;
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") total = e.data.contentLength ?? 0;
        else if (e.event === "Progress") done += e.data.chunkLength;
        setState({ kind: "downloading", update, done, total });
      });
      setState({ kind: "ready", update });
    } catch (e) {
      setState({ kind: "install-failed", update, message: errorText(e) });
    }
  };

  const progress = state.kind === "downloading" && state.total ? state.done / state.total : state.kind === "ready" ? 1 : 0;

  return (
    <div className="flex h-screen flex-col bg-bg text-fg">
      <div data-tauri-drag-region className={cn("flex h-12 shrink-0 items-center gap-2 pr-3", isMac ? "pl-[78px]" : "pl-4")}>
        <img src={appIcon} alt="" className="pointer-events-none h-[18px] w-[18px] rounded-[5px]" draggable={false} />
        <span className="pointer-events-none flex-1 truncate text-[13.5px] font-semibold tracking-[0.01em]">Mihomo-Monitor</span>
        {!isMac && <WindowControls />}
      </div>
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto px-6 pb-10">
        <div className="w-full max-w-[520px] rounded-2xl bg-card p-6 shadow-card">
          <span className="flex h-10 w-10 items-center justify-center rounded-xl bg-warn-soft text-warn">
            <DatabaseZap className="h-5 w-5" />
          </span>
          <h1 className="mt-3 text-[16px] font-semibold">需要更新 Mihomo-Monitor</h1>
          <p className="mt-1.5 text-[12.5px] leading-5 text-fg-2">
            这台电脑上的统计数据由更新版本的 Mihomo-Monitor 写入（数据版本 {data.found}，当前版本最高只能读取 {data.supported}）。
            为了不损坏这些数据，当前版本不会打开它们，也不会开始统计。更新到最新版本后即可继续，数据不会丢失。
          </p>

          <div className="mt-4 rounded-xl bg-card-2 px-4 py-3 text-[12.5px] shadow-edge">
            {state.kind === "checking" && (
              <span className="flex items-center gap-2 text-muted">
                <Loader2 className="h-4 w-4 animate-spin" />
                正在检查更新…
              </span>
            )}
            {state.kind === "none" && (
              <p className="leading-5 text-fg-2">
                没有找到比当前更新的正式版本。这些数据可能来自开发版本：可以等新版本发布后再打开这里更新，或者继续使用开发版本。
              </p>
            )}
            {state.kind === "failed" && <p className="selectable text-danger">检查更新失败：{state.message}</p>}
            {(state.kind === "found" || state.kind === "downloading" || state.kind === "ready" || state.kind === "install-failed") && (
              <div className="space-y-2.5">
                <div className="font-medium">v{state.update.version} 可以更新</div>
                {state.update.body && (
                  <div className="max-h-[160px] overflow-y-auto text-[12px]">
                    <ReleaseNotes releases={[{ version: state.update.version, date: state.update.date ?? "", body: state.update.body }]} />
                  </div>
                )}
                {(state.kind === "downloading" || state.kind === "ready") && (
                  <div className="space-y-1.5">
                    <div className="h-1.5 overflow-hidden rounded-full bg-pill">
                      <div className="h-full rounded-full bg-brand transition-[width] duration-200" style={{ width: `${Math.round(progress * 100)}%` }} />
                    </div>
                    <p className="flex items-center gap-1.5 text-[11.5px] text-muted">
                      {state.kind === "ready" ? (
                        <>
                          <Check className="h-3.5 w-3.5 text-ok" />
                          已下载并安装，重启后生效。
                        </>
                      ) : state.total ? (
                        <>
                          正在下载 {mb(state.done)} / {mb(state.total)} MB · {Math.round(progress * 100)}%
                        </>
                      ) : (
                        <>
                          <Loader2 className="h-3 w-3 animate-spin" />
                          正在准备下载…
                        </>
                      )}
                    </p>
                  </div>
                )}
                {state.kind === "install-failed" && <p className="selectable text-danger">更新失败：{state.message}</p>}
              </div>
            )}
          </div>

          <div className="mt-5 flex flex-wrap items-center justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={() => void api.openDataDir().catch(() => {})}>
              <FolderOpen />
              打开数据目录
            </Button>
            <Button variant="ghost" size="sm" onClick={() => void api.quitApp()}>
              退出
            </Button>
            {(state.kind === "none" || state.kind === "failed") && (
              <Button size="sm" variant="outline" onClick={() => void look()}>
                <RefreshCw />
                重新检查
              </Button>
            )}
            {(state.kind === "found" || state.kind === "install-failed") && (
              <Button size="sm" onClick={() => void install(state.update)}>
                <ArrowDownToLine />
                下载并安装 v{state.update.version}
              </Button>
            )}
            {state.kind === "downloading" && (
              <Button size="sm" disabled>
                <Loader2 className="animate-spin" />
                正在更新
              </Button>
            )}
            {state.kind === "ready" && (
              <Button size="sm" onClick={() => void api.restartApp()}>
                重启 Mihomo-Monitor
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
