import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { bytes, cn, niceBytesMax, rate } from "@/lib/utils";

/** Series of every traffic chart, in legend and stacking order (download at the base). */
export const SERIES = [
  { key: "down", label: "下载", color: "rgb(var(--down))", wash: "rgb(var(--down) / 0.1)" },
  { key: "up", label: "上传", color: "rgb(var(--up))", wash: "rgb(var(--up) / 0.1)" },
] as const;

const AXIS = { fill: "rgb(var(--muted))" };
const GRID = { stroke: "rgb(var(--line-2))" };

function useWidth(min = 240) {
  const box = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(600);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(min, e.contentRect.width)));
    ro.observe(el);
    return () => ro.disconnect();
  }, [min]);
  return [box, width] as const;
}

/** Short stroke keys naming the series; always shown, so identity never rests on color alone. */
export function Legend({ className }: { className?: string }) {
  return (
    <div className={cn("flex items-center gap-3 text-[11.5px] text-muted", className)}>
      {SERIES.map((s) => (
        <span key={s.key} className="inline-flex items-center gap-1.5">
          <span className="h-[2px] w-3 rounded-full" style={{ background: s.color }} />
          {s.label}
        </span>
      ))}
    </div>
  );
}

function Tooltip({ x, width, title, rows }: { x: number; width: number; title: ReactNode; rows: { key: string; label: string; color?: string; value: string }[] }) {
  const w = 168;
  const left = Math.min(Math.max(x - w / 2, 0), width - w);
  return (
    <div
      role="status"
      style={{ left, width: w, background: "var(--pop)" }}
      className="pointer-events-none absolute top-0 z-10 rounded-lg px-2.5 py-2 text-[11.5px] shadow-pop backdrop-blur-md"
    >
      <div className="mb-1 text-muted">{title}</div>
      {rows.map((r) => (
        <div key={r.key} className="flex items-center justify-between gap-3 leading-5">
          <span className="inline-flex items-center gap-1.5 text-fg-2">
            {r.color && <span className="h-[2px] w-2.5 rounded-full" style={{ background: r.color }} />}
            {r.label}
          </span>
          <span className="font-medium tabular-nums text-fg">{r.value}</span>
        </div>
      ))}
    </div>
  );
}

export interface RatePoint {
  /** Unix seconds. */
  at: number;
  up: number | null;
  down: number | null;
}

const LIVE_H = 150;
const LIVE_PAD = { top: 10, right: 8, bottom: 22, left: 64 };

/** Per-second download and upload rates as two lines, with a crosshair readout. */
export function LiveChart({ points }: { points: RatePoint[] }) {
  const [box, width] = useWidth();
  const [active, setActive] = useState<number | null>(null);
  const n = points.length;
  const top = niceBytesMax(Math.max(0, ...points.flatMap((p) => [p.up ?? 0, p.down ?? 0])));
  const plotW = width - LIVE_PAD.left - LIVE_PAD.right;
  const plotH = LIVE_H - LIVE_PAD.top - LIVE_PAD.bottom;
  const x = (i: number) => LIVE_PAD.left + (n > 1 ? (i / (n - 1)) * plotW : plotW);
  const y = (v: number) => LIVE_PAD.top + plotH - (v / top) * plotH;
  const base = y(0);

  // A missing second (core offline) breaks the line instead of drawing a fake slope.
  const paths = SERIES.map((s) => {
    let line = "";
    let area = "";
    let run: number[] = [];
    const close = () => {
      if (run.length) area += `L${x(run[run.length - 1]).toFixed(1)},${base}L${x(run[0]).toFixed(1)},${base}Z`;
      run = [];
    };
    points.forEach((p, i) => {
      const v = p[s.key];
      if (v === null) return close();
      const cmd = `${x(i).toFixed(1)},${y(v).toFixed(1)}`;
      line += run.length ? `L${cmd}` : `M${cmd}`;
      area += run.length ? `L${cmd}` : `M${cmd}`;
      run.push(i);
    });
    close();
    return { ...s, line, area };
  });

  const pick = (clientX: number) => {
    const rect = box.current?.getBoundingClientRect();
    if (!rect || n < 2) return;
    const i = Math.round(((clientX - rect.left - LIVE_PAD.left) / plotW) * (n - 1));
    setActive(Math.min(n - 1, Math.max(0, i)));
  };
  const minutes = n > 1 ? Math.round((points[n - 1].at - points[0].at) / 60) : 0;
  const shown = active !== null ? points[active] : null;

  return (
    <div ref={box} className="relative">
      <svg
        width={width}
        height={LIVE_H}
        role="img"
        aria-label={`最近 ${minutes} 分钟的下载与上传速率`}
        className="block"
        onPointerMove={(e) => pick(e.clientX)}
        onPointerLeave={() => setActive(null)}
      >
        {[0, 0.5, 1].map((f) => (
          <line key={f} x1={LIVE_PAD.left} x2={width - LIVE_PAD.right} y1={y(top * f)} y2={y(top * f)} strokeWidth={1} style={GRID} />
        ))}
        {[0, 0.5, 1].map((f) => (
          <text key={f} x={LIVE_PAD.left - 8} y={y(top * f) + 3.5} textAnchor="end" fontSize={10.5} className="tabular-nums" style={AXIS}>
            {rate(top * f)}
          </text>
        ))}
        <text x={LIVE_PAD.left} y={LIVE_H - 5} fontSize={10.5} style={AXIS}>
          {minutes} 分钟前
        </text>
        <text x={width - LIVE_PAD.right} y={LIVE_H - 5} textAnchor="end" fontSize={10.5} style={AXIS}>
          现在
        </text>
        {paths.map((p) => (
          <g key={p.key}>
            <path d={p.area} style={{ fill: p.wash }} />
            <path d={p.line} fill="none" style={{ stroke: p.color }} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
          </g>
        ))}
        {active !== null && (
          <>
            <line x1={x(active)} x2={x(active)} y1={LIVE_PAD.top} y2={base} strokeWidth={1} style={{ stroke: "rgb(var(--muted))" }} />
            {SERIES.map((s) => {
              const v = points[active][s.key];
              return v === null ? null : (
                <circle key={s.key} cx={x(active)} cy={y(v)} r={4} strokeWidth={2} style={{ fill: s.color, stroke: "rgb(var(--card))" }} />
              );
            })}
          </>
        )}
      </svg>
      {shown && active !== null && (
        <Tooltip
          x={x(active)}
          width={width}
          title={new Date(shown.at * 1000).toLocaleTimeString("zh-CN")}
          rows={SERIES.map((s) => ({ key: s.key, label: s.label, color: s.color, value: shown[s.key] === null ? "无数据" : rate(shown[s.key] ?? 0) }))}
        />
      )}
    </div>
  );
}

export interface BarItem {
  key: string;
  /** Axis label; empty to leave the slot unlabeled. */
  label: string;
  /** Tooltip and table heading. */
  title: string;
  up: number;
  down: number;
}

const BAR_H = 180;
const BAR_PAD = { top: 10, right: 8, bottom: 22, left: 64 };
const GAP = 2;
const RADIUS = 4;

/** A column with its top corners rounded and its base square, as columns grow from the axis. */
function column(x: number, y: number, w: number, h: number, r: number) {
  if (h <= 0) return "";
  const k = Math.min(r, h, w / 2);
  return `M${x},${y + h}V${y + k}Q${x},${y} ${x + k},${y}H${x + w - k}Q${x + w},${y} ${x + w},${y + k}V${y + h}Z`;
}

/**
 * Download and upload stacked per period, with a per-column readout and, unless the page shows
 * its own table (`tableToggle={false}`), a table view.
 */
export function StackedBars({ items, ariaLabel, tableToggle = true }: { items: BarItem[]; ariaLabel: string; tableToggle?: boolean }) {
  const [box, width] = useWidth();
  const [active, setActive] = useState<number | null>(null);
  const [table, setTable] = useState(false);
  const top = niceBytesMax(Math.max(0, ...items.map((d) => d.up + d.down)));
  const plotW = width - BAR_PAD.left - BAR_PAD.right;
  const plotH = BAR_H - BAR_PAD.top - BAR_PAD.bottom;
  const slot = plotW / Math.max(1, items.length);
  const barW = Math.min(24, Math.max(2, slot - 4));
  const y = (v: number) => BAR_PAD.top + plotH - (v / top) * plotH;
  const h = (v: number) => (v / top) * plotH;
  const base = y(0);
  const hovered = active !== null ? items[active] : null;

  return (
    <div>
      <div className="mb-2 flex items-center justify-between gap-3">
        <Legend />
        {tableToggle && (
          <button className="shrink-0 text-[11.5px] text-muted hover:text-fg active:!scale-100" onClick={() => setTable((t) => !t)}>
            {table ? "图表" : "表格"}
          </button>
        )}
      </div>
      {table ? (
        <div className="max-h-[220px] overflow-y-auto rounded-lg border border-line-2">
          <table className="w-full text-[11.5px] tabular-nums">
            <thead className="sticky top-0 bg-card text-muted">
              <tr>
                {["时间", "下载", "上传", "合计"].map((t) => (
                  <th key={t} className="px-2.5 py-1.5 text-right font-normal first:text-left">
                    {t}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="text-fg-2">
              {[...items].reverse().map((d) => (
                <tr key={d.key} className="border-t border-line-2">
                  <td className="px-2.5 py-1">{d.title}</td>
                  {[d.down, d.up, d.down + d.up].map((v, i) => (
                    <td key={i} className="px-2.5 py-1 text-right">
                      {bytes(v)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div ref={box} className="relative">
          <svg width={width} height={BAR_H} role="img" aria-label={ariaLabel} className="block" onPointerLeave={() => setActive(null)}>
            {[0, 0.5, 1].map((f) => (
              <line key={f} x1={BAR_PAD.left} x2={width - BAR_PAD.right} y1={y(top * f)} y2={y(top * f)} strokeWidth={1} style={GRID} />
            ))}
            {[0, 0.5, 1].map((f) => (
              <text key={f} x={BAR_PAD.left - 8} y={y(top * f) + 3.5} textAnchor="end" fontSize={10.5} className="tabular-nums" style={AXIS}>
                {bytes(top * f)}
              </text>
            ))}
            {items.map((d, i) => {
              const cx = BAR_PAD.left + slot * i + slot / 2;
              const x0 = cx - barW / 2;
              const downH = h(d.down);
              const upH = h(d.up);
              // The gap between the two parts comes out of the upper one, so the total height stays true.
              const upVisible = upH > GAP ? upH - GAP : 0;
              const dim = active !== null && active !== i;
              return (
                <g key={d.key} opacity={dim ? 0.45 : 1}>
                  {downH > 0 && <path d={upVisible ? `M${x0},${base}V${base - downH}H${x0 + barW}V${base}Z` : column(x0, base - downH, barW, downH, RADIUS)} style={{ fill: SERIES[0].color }} />}
                  {upVisible > 0 && <path d={column(x0, base - downH - upH, barW, upVisible, RADIUS)} style={{ fill: SERIES[1].color }} />}
                  {d.label && (
                    <text x={cx} y={BAR_H - 5} textAnchor="middle" fontSize={10.5} style={AXIS}>
                      {d.label}
                    </text>
                  )}
                  {/* The whole column slot is the hit target, not just the (possibly tiny) bar. */}
                  <rect
                    x={BAR_PAD.left + slot * i}
                    y={BAR_PAD.top}
                    width={slot}
                    height={plotH}
                    fill="transparent"
                    tabIndex={0}
                    aria-label={`${d.title}：下载 ${bytes(d.down)}，上传 ${bytes(d.up)}`}
                    onPointerEnter={() => setActive(i)}
                    onFocus={() => setActive(i)}
                    onBlur={() => setActive(null)}
                    className="outline-none"
                  />
                </g>
              );
            })}
            <line x1={BAR_PAD.left} x2={width - BAR_PAD.right} y1={base} y2={base} strokeWidth={1} style={{ stroke: "rgb(var(--line))" }} />
          </svg>
          {hovered && active !== null && (
            <Tooltip
              x={BAR_PAD.left + slot * active + slot / 2}
              width={width}
              title={hovered.title}
              rows={[
                ...SERIES.map((s) => ({ key: s.key, label: s.label, color: s.color, value: bytes(hovered[s.key]) })),
                { key: "sum", label: "合计", value: bytes(hovered.up + hovered.down) },
              ]}
            />
          )}
        </div>
      )}
    </div>
  );
}
