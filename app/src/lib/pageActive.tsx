import { createContext, useContext, useEffect, useRef, type ReactNode } from "react";

/**
 * Pages stay mounted once opened and are only hidden when another one is shown, so going back
 * to one shows what it had at once while it reads again in the background.
 */
const Active = createContext(true);

export function PageSlot({ active, children }: { active: boolean; children: ReactNode }) {
  return (
    <div hidden={!active}>
      <Active.Provider value={active}>{children}</Active.Provider>
    </div>
  );
}

/** Whether the page is the one shown; a hidden page stops its timers and listeners. */
export function usePageActive(): boolean {
  return useContext(Active);
}

/** Calls `refresh` each time the page is shown again after being hidden (not when first opened). */
export function useOnShow(refresh: () => void) {
  const active = usePageActive();
  const latest = useRef(refresh);
  latest.current = refresh;
  const shownBefore = useRef(active);
  useEffect(() => {
    if (active && !shownBefore.current) latest.current();
    shownBefore.current = active;
  }, [active]);
}
