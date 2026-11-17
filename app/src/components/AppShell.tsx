import { UpdateNotice } from "./UpdateNotice";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CalendarRange, ChartPie, Cpu, LayoutDashboard, Tags, Ticket, Info, Minus, Moon, Settings, Square, Sun, SunMoon, X } from "lucide-react";
import type React from "react";
import { useLayoutEffect, useRef, useState } from "react";
import appIcon from "@/assets/app-icon.png";
import type { ThemeMode } from "@/lib/theme";
import { cn } from "@/lib/utils";

export type PageId = "overview" | "history" | "breakdown" | "groups" | "subscriptions" | "sources" | "settings" | "about";

type Icon = React.ComponentType<{ className?: string }>;
interface NavItem { id: PageId; label: string; icon: Icon }

const isMac = navigator.userAgent.includes("Mac");
const main: NavItem[] = [
  { id: "overview", label: "概览", icon: LayoutDashboard },
  { id: "history", label: "历史", icon: CalendarRange },
  { id: "breakdown", label: "分布", icon: ChartPie },
  { id: "groups", label: "域名分组", icon: Tags },
  { id: "subscriptions", label: "订阅", icon: Ticket },
  { id: "sources", label: "内核", icon: Cpu },
];
const footer: NavItem[] = [
  { id: "settings", label: "设置", icon: Settings },
  { id: "about", label: "关于", icon: Info },
];
const titles: Record<PageId, string> = { overview: "概览", history: "历史", breakdown: "分布", groups: "域名分组", subscriptions: "订阅", sources: "内核", settings: "设置", about: "关于" };

const nextTheme: Record<ThemeMode, ThemeMode> = { system: "light", light: "dark", dark: "system" };
const themeLabel: Record<ThemeMode, string> = { system: "跟随系统", light: "浅色", dark: "深色" };

interface AppShellProps {
  activePage: PageId;
  onPageChange: (page: PageId) => void;
  theme: ThemeMode;
  onThemeChange: (mode: ThemeMode) => void;
  /** Cores being read right now, and cores known; null while loading. */
  status?: { online: number; known: number } | null;
  /** Controls placed in the title bar, left of the update and theme buttons. */
  headerExtra?: React.ReactNode;
  children: React.ReactNode;
}

/** Minimise, maximise and close for the frameless window off macOS (macOS has its traffic lights). */
export function WindowControls() {
  const win = getCurrentWindow();
  const base = "flex h-12 w-11 items-center justify-center text-muted hover:text-fg";
  return (
    <div className="-mr-3 ml-1 flex">
      <button className={cn(base, "hover:bg-pill")} aria-label="最小化" onClick={() => win.minimize()}><Minus className="h-4 w-4" /></button>
      <button className={cn(base, "hover:bg-pill")} aria-label="最大化" onClick={() => win.toggleMaximize()}><Square className="h-3 w-3" /></button>
      <button className={cn(base, "hover:bg-[#e81123] hover:text-white")} aria-label="关闭到托盘" title="关闭窗口（Mihomo-Monitor 留在托盘继续统计）" onClick={() => win.close()}><X className="h-4 w-4" /></button>
    </div>
  );
}

function NavRow({ item, active, onSelect, setRef }: { item: NavItem; active: boolean; onSelect: () => void; setRef: (el: HTMLButtonElement | null) => void }) {
  const Icon = item.icon;
  return (
    <button
      ref={setRef}
      onClick={onSelect}
      aria-current={active ? "page" : undefined}
      className={cn(
        "relative z-10 flex h-[30px] w-full items-center gap-[9px] rounded-[7px] px-2 text-left text-[13px] active:!scale-100",
        active ? "font-medium text-fg" : "text-fg-2 hover:bg-pill hover:text-fg",
      )}
    >
      <Icon className={cn("h-[15px] w-[15px] shrink-0", active ? "text-fg" : "text-muted")} />
      {item.label}
    </button>
  );
}

export function AppShell({ activePage, onPageChange, theme, onThemeChange, status, headerExtra, children }: AppShellProps) {
  const ThemeIcon = theme === "light" ? Sun : theme === "dark" ? Moon : SunMoon;
  const refs = useRef<Partial<Record<PageId, HTMLButtonElement | null>>>({});
  const nav = useRef<HTMLDivElement>(null);
  const [thumb, setThumb] = useState<number | null>(null);

  // Sliding highlight behind the active row.
  useLayoutEffect(() => {
    const measure = () => {
      const el = refs.current[activePage];
      const box = nav.current;
      if (el && box) setThumb(el.getBoundingClientRect().top - box.getBoundingClientRect().top);
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [activePage]);

  const row = (item: NavItem) => (
    <NavRow key={item.id} item={item} active={activePage === item.id} onSelect={() => onPageChange(item.id)} setRef={(el) => (refs.current[item.id] = el)} />
  );

  return (
    <div className="flex h-screen overflow-hidden bg-bg text-fg">
      <aside className="flex w-[228px] shrink-0 flex-col border-r-[0.5px] border-line bg-bg">
        <div data-tauri-drag-region className={cn("flex h-12 shrink-0 items-center gap-2", isMac ? "pl-[78px]" : "pl-4")}>
          <img src={appIcon} alt="" className="pointer-events-none h-[18px] w-[18px] rounded-[5px]" draggable={false} />
          <span className="pointer-events-none truncate text-[13.5px] font-semibold tracking-[0.01em]">Mihomo-Monitor</span>
        </div>

        <div ref={nav} className="relative flex min-h-0 flex-1 flex-col px-2.5">
          {thumb !== null && (
            <span
              aria-hidden
              className="absolute inset-x-2.5 h-[30px] rounded-[7px] bg-card shadow-edge transition-[top] duration-300 ease-soft dark:bg-white/[.07]"
              style={{ top: thumb }}
            />
          )}
          <div className="flex-1 space-y-0.5 overflow-y-auto pb-2 pt-1">{main.map(row)}</div>
          <div className="space-y-0.5 border-t-[0.5px] border-line-2 py-2">{footer.map(row)}</div>
        </div>

        <div className="flex h-9 shrink-0 items-center gap-2 px-[18px] pb-1 text-[11.5px] text-muted">
          <span className={cn("h-[7px] w-[7px] rounded-full", status?.online ? "bg-ok shadow-[0_0_0_3px_rgb(var(--ok-soft))]" : "bg-faint")} />
          {!status ? "读取中…" : status.online ? `正在统计 ${status.online} 个内核` : "未连接到内核"}
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header data-tauri-drag-region className="flex h-12 shrink-0 items-center justify-between gap-3 pl-5 pr-3">
          <div data-tauri-drag-region className="pointer-events-none min-w-0 truncate text-[13.5px] font-semibold">{titles[activePage]}</div>
          <div className="flex items-center gap-1">
            {headerExtra}
            <UpdateNotice />
            <button
              onClick={() => onThemeChange(nextTheme[theme])}
              title={`主题：${themeLabel[theme]}（点击切换）`}
              aria-label={`主题：${themeLabel[theme]}`}
              className="flex h-[26px] w-[26px] items-center justify-center rounded-[7px] text-muted hover:bg-pill hover:text-fg"
            >
              <ThemeIcon className="h-[15px] w-[15px]" />
            </button>
            {!isMac && <WindowControls />}
          </div>
        </header>
        <main className="min-h-0 flex-1 overflow-y-auto">{children}</main>
      </div>
    </div>
  );
}
