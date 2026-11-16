import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { AppShell, type PageId } from "@/components/AppShell";
import { api, type AppInfo, type Source } from "@/lib/api";
import { useTheme } from "@/lib/theme";
import { AboutPage } from "@/pages/AboutPage";
import { BreakdownPage } from "@/pages/BreakdownPage";
import { HistoryPage } from "@/pages/HistoryPage";
import { OverviewPage } from "@/pages/OverviewPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { SourcesPage } from "@/pages/SourcesPage";

export default function App() {
  const [page, setPage] = useState<PageId>("overview");
  const [theme, setTheme] = useTheme();
  const [sources, setSources] = useState<Source[] | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSources(await api.listSources());
    } catch {
      /* shown as still loading; the next change event retries */
    }
  }, []);

  useEffect(() => {
    void refresh();
    void api.appInfo().then(setInfo).catch(() => {});
    const un = listen("sources-changed", () => void refresh());
    return () => void un.then((f) => f());
  }, [refresh]);

  const status = sources ? { online: sources.filter((s) => s.status === "online").length, known: sources.length } : null;

  return (
    <AppShell activePage={page} onPageChange={setPage} theme={theme} onThemeChange={setTheme} status={status}>
      {page === "overview" && <OverviewPage sources={sources} onOpenSources={() => setPage("sources")} />}
      {page === "history" && <HistoryPage sources={sources} />}
      {page === "breakdown" && <BreakdownPage sources={sources} />}
      {page === "sources" && <SourcesPage sources={sources} />}
      {page === "settings" && <SettingsPage info={info} />}
      {page === "about" && <AboutPage info={info} />}
    </AppShell>
  );
}
