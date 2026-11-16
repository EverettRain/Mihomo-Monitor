import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { DayPoint } from "@/lib/api";
import { bytes } from "@/lib/utils";

const WEEKS = 53;
const GAP = 3;
const LEFT = 22;
const BOTTOM = 18;
const WEEKDAYS_MON = ["一", "", "三", "", "五", "", ""];
const WEEKDAYS_SUN = ["日", "", "二", "", "四", "", "六"];
/** One hue, light to dark: the step's opacity over the card. */
const STEPS = [0.3, 0.5, 0.75, 1];

const total = (d: DayPoint | undefined) => (d ? d.up + d.down : 0);

/** Quartile thresholds of the non-zero values, so a single huge day does not wash out the rest. */
function thresholds(values: number[]): number[] {
  const sorted = values.filter((v) => v > 0).sort((a, b) => a - b);
  if (!sorted.length) return [];
  return [0.25, 0.5, 0.75].map((q) => sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]);
}

/**
 * A year of daily traffic, one column per week. `days` ends today, oldest first; the first
 * column starts on the week's first day, so a week's days line up in a column.
 */
export function Heatmap({ days, weekStartsSunday }: { days: DayPoint[]; weekStartsSunday: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(760);
  const [hover, setHover] = useState<DayPoint | null>(null);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(e.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const cell = Math.max(6, Math.min(14, Math.floor((width - LEFT) / WEEKS) - GAP));
  const step = cell + GAP;

  const grid = useMemo(() => {
    if (!days.length) return [];
    // Weekday of today (0 = the week's first day) fixes how many days the last column holds.
    const today = new Date(`${days[days.length - 1].date}T00:00:00`);
    const todayRow = weekStartsSunday ? today.getDay() : (today.getDay() + 6) % 7;
    const shown = days.slice(Math.max(0, days.length - ((WEEKS - 1) * 7 + todayRow + 1)));
    return shown.map((d, i) => {
      const [, m, day] = d.date.split("-").map(Number);
      return { d, col: Math.floor(i / 7), row: i % 7, month: m, date: day };
    });
  }, [days, weekStartsSunday]);

  const limits = useMemo(() => thresholds(grid.map((g) => total(g.d))), [grid]);
  const level = (v: number) => (v <= 0 ? -1 : limits.filter((t) => v > t).length);
  const months = grid.filter((g) => g.row === 0 && g.date <= 7);
  const weekdays = weekStartsSunday ? WEEKDAYS_SUN : WEEKDAYS_MON;

  return (
    <div ref={box}>
      <div className="mb-1.5 h-4 px-0.5 text-[11.5px] text-muted">
        {hover ? (
          <>
            <span className="font-mono tabular-nums text-fg">{bytes(total(hover))}</span> · {hover.date} · 下载 {bytes(hover.down)} · 上传 {bytes(hover.up)}
          </>
        ) : (
          "悬停查看每天的流量"
        )}
      </div>
      <svg width={LEFT + WEEKS * step} height={7 * step + BOTTOM} role="img" aria-label="最近一年每天的流量热力图" onPointerLeave={() => setHover(null)}>
        {weekdays.map(
          (w, i) =>
            w && (
              <text key={i} x={0} y={i * step + cell - 2} fontSize={10} style={{ fill: "rgb(var(--muted))" }}>
                {w}
              </text>
            ),
        )}
        {grid.map((g) => {
          const lv = level(total(g.d));
          return (
            <rect
              key={g.d.date}
              x={LEFT + g.col * step}
              y={g.row * step}
              width={cell}
              height={cell}
              rx={2.5}
              style={{
                fill: lv < 0 ? "rgb(var(--pill))" : `rgb(var(--accent) / ${STEPS[lv]})`,
                stroke: hover?.date === g.d.date ? "rgb(var(--fg))" : "none",
                strokeWidth: 1.5,
              }}
              onPointerEnter={() => setHover(g.d)}
            >
              <title>{`${g.d.date}：${bytes(total(g.d))}`}</title>
            </rect>
          );
        })}
        {months.map((g) => (
          <text key={g.d.date} x={LEFT + g.col * step} y={7 * step + 12} fontSize={10.5} style={{ fill: "rgb(var(--muted))" }}>
            {g.month}月
          </text>
        ))}
      </svg>
      <div className="mt-1 flex items-center justify-end gap-1 text-[10.5px] text-muted">
        少
        {[-1, 0, 1, 2, 3].map((lv) => (
          <span key={lv} className="inline-block h-2.5 w-2.5 rounded-[2px]" style={{ background: lv < 0 ? "rgb(var(--pill))" : `rgb(var(--accent) / ${STEPS[lv]})` }} />
        ))}
        多
      </div>
    </div>
  );
}
