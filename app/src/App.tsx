import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { AppShell, type PageId } from "@/components/AppShell";
import { api, type AppInfo, type NewerData, type Source } from "@/lib/api";
import { PageSlot } from "@/lib/pageActive";
import { useTheme } from "@/lib/theme";
import { AboutPage } from "@/pages/AboutPage";
import { BreakdownPage } from "@/pages/BreakdownPage";
import { AttributionPage } from "@/pages/AttributionPage";
import { GroupsPage } from "@/pages/GroupsPage";
import { HistoryPage } from "@/pages/HistoryPage";
import { NewerDataPage } from "@/pages/NewerDataPage";
import { OverviewPage } from "@/pages/OverviewPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { SourcesPage } from "@/pages/SourcesPage";
import { SubscriptionsPage } from "@/pages/SubscriptionsPage";

/**
 * Asks first whether the data on disk comes from a later version: then only updating is offered,
 * and none of the app's other commands are called.
 */
export default function App() {
  const [newer, setNewer] = useState<NewerData | null | undefined>(undefined);
  useEffect(() => {
    api.newerData().then(setNewer, () => setNewer(null));
  }, []);
  if (newer === undefined) return null;
  return newer ? <NewerDataPage data={newer} /> : <Main />;
}

function Main() {
  const [page, setPage] = useState<PageId>("overview");
  // Pages opened once stay mounted (hidden when not shown), so going back is instant.
  const [opened, setOpened] = useState<ReadonlySet<PageId>>(() => new Set<PageId>(["overview"]));
  const open = useCallback((p: PageId) => {
    setOpened((o) => (o.has(p) ? o : new Set(o).add(p)));
    setPage(p);
  }, []);
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
    <AppShell activePage={page} onPageChange={open} theme={theme} onThemeChange={setTheme} status={status}>
      {(
        [
          ["overview", <OverviewPage sources={sources} onOpenSources={() => open("sources")} onOpenSubscriptions={() => open("subscriptions")} />],
          ["history", <HistoryPage sources={sources} />],
          ["breakdown", <BreakdownPage sources={sources} />],
          ["groups", <GroupsPage />],
          ["rules", <AttributionPage />],
          ["subscriptions", <SubscriptionsPage />],
          ["sources", <SourcesPage sources={sources} />],
          ["settings", <SettingsPage info={info} />],
          ["about", <AboutPage info={info} />],
        ] as const
      ).map(
        ([id, node]) =>
          opened.has(id) && (
            <PageSlot key={id} active={page === id}>
              {node}
            </PageSlot>
          ),
      )}
    </AppShell>
  );
}
