import type { ThemeChoice } from "@/lib/prefs";
import { Sun, Moon, Monitor } from "@/lib/icons";
import { cn } from "@/lib/utils";

/**
 * The swatch inside each option is painted from literal hex rather than the
 * theme tokens on purpose: all three previews have to show their OWN palette
 * at the same time, and a token resolves to whichever mode the app is
 * currently in. These are the `--background` / `--titlebar` / `--card` values
 * from globals.css; if that palette moves, move these with it.
 */
const PALETTES = {
  light: { bg: "#fbfbf9", bar: "#f5f5f2", card: "#ffffff", ink: "#23262a", line: "#e7e7e2" },
  dark: { bg: "#17191c", bar: "#1b1d21", card: "#282b30", ink: "#e6e9ed", line: "#2a2d33" },
} as const;

function Preview({ mode }: { mode: "light" | "dark" }) {
  const p = PALETTES[mode];
  return (
    <svg viewBox="0 0 72 48" className="h-full w-full" aria-hidden="true">
      <rect x="0" y="0" width="72" height="48" rx="5" fill={p.bg} />
      <path d="M0 5a5 5 0 0 1 5-5h62a5 5 0 0 1 5 5v6H0Z" fill={p.bar} />
      <rect x="5" y="4.4" width="16" height="2.4" rx="1.2" fill={p.ink} opacity="0.5" />
      <rect x="10" y="17" width="52" height="24" rx="4" fill={p.card} stroke={p.line} />
      <rect x="16" y="23" width="26" height="3" rx="1.5" fill={p.ink} opacity="0.75" />
      <rect x="16" y="29.5" width="40" height="2.4" rx="1.2" fill={p.ink} opacity="0.3" />
      <rect x="16" y="34.5" width="33" height="2.4" rx="1.2" fill={p.ink} opacity="0.3" />
    </svg>
  );
}

/** The "system" tile shows both palettes, split down a diagonal. */
function SystemPreview() {
  return (
    <div className="relative h-full w-full">
      <Preview mode="light" />
      <div
        className="absolute inset-0"
        style={{ clipPath: "polygon(100% 0, 100% 100%, 0 100%)" }}
      >
        <Preview mode="dark" />
      </div>
    </div>
  );
}

const OPTIONS: { value: ThemeChoice; label: string; Icon: typeof Sun }[] = [
  { value: "light", label: "Light", Icon: Sun },
  { value: "dark", label: "Dark", Icon: Moon },
  { value: "system", label: "System", Icon: Monitor },
];

export function ThemePicker({
  value,
  onChange,
}: {
  value: ThemeChoice;
  onChange: (theme: ThemeChoice) => void;
}) {
  return (
    <div role="radiogroup" aria-label="Appearance" className="grid grid-cols-3 gap-3">
      {OPTIONS.map(({ value: option, label, Icon }) => {
        const selected = value === option;
        return (
          <button
            key={option}
            type="button"
            role="radio"
            aria-checked={selected}
            onClick={() => onChange(option)}
            className={cn(
              "group flex flex-col gap-2.5 rounded-md p-2.5 text-left outline-none",
              "transition-colors duration-150 ease-out",
              "ring-1 focus-visible:ring-2 focus-visible:ring-ring",
              selected
                ? "card-surface-raised ring-border-strong"
                : "card-surface-subtle ring-transparent hover:fill-control-hover",
            )}
          >
            <div className="aspect-[3/2] overflow-hidden rounded-sm">
              {option === "system" ? <SystemPreview /> : <Preview mode={option} />}
            </div>
            <div className="flex items-center gap-1.5 px-0.5">
              <Icon
                className={cn(
                  "size-3.5 transition-colors duration-150",
                  selected ? "text-foreground" : "text-muted-foreground",
                )}
              />
              <span
                className={cn(
                  "text-xs font-medium transition-colors duration-150",
                  selected ? "text-foreground" : "text-muted-foreground",
                )}
              >
                {label}
              </span>
            </div>
          </button>
        );
      })}
    </div>
  );
}
