import { cn } from "@/lib/utils";

/** A row of mutually exclusive choices, as in Hydra's dashboard. */
export function Segmented<T extends string | number>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: readonly (readonly [T, string])[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex min-w-0 flex-wrap gap-0.5 rounded-lg bg-pill p-0.5">
      {options.map(([v, text]) => (
        <button
          key={String(v)}
          role="radio"
          aria-checked={value === v}
          onClick={() => onChange(v)}
          className={cn(
            "h-6 max-w-[160px] truncate rounded-md px-2.5 text-[12px] active:!scale-100",
            value === v ? "bg-card font-medium text-fg shadow-edge dark:bg-white/[.12]" : "text-muted hover:text-fg",
          )}
        >
          {text}
        </button>
      ))}
    </div>
  );
}
