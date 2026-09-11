import { useCallback, useEffect, useState } from "react";
import { Mark } from "@/onboarding/Mark";
import { ThemePicker } from "@/onboarding/ThemePicker";
import { ArrowLeft, ArrowRight, Check, Copy, Sparkles, Square } from "@/lib/icons";
import { applyTheme, type Preferences, type ThemeChoice } from "@/lib/prefs";
import { cn } from "@/lib/utils";

type StepId = "welcome" | "appearance" | "chrome" | "ready";

const STEPS: StepId[] = ["welcome", "appearance", "chrome", "ready"];

export function Onboarding({
  preferences,
  onChange,
  onFinish,
}: {
  preferences: Preferences;
  /** Persists as the user goes, so a mid-walkthrough quit keeps the theme. */
  onChange: (patch: Partial<Preferences>) => void;
  onFinish: () => void;
}) {
  const [index, setIndex] = useState(0);
  const step = STEPS[index];
  const isLast = index === STEPS.length - 1;

  const back = useCallback(() => setIndex((i) => Math.max(0, i - 1)), []);
  const next = useCallback(() => {
    if (isLast) onFinish();
    else setIndex((i) => i + 1);
  }, [isLast, onFinish]);

  // Enter and the arrow keys drive the flow. Worth wiring explicitly rather
  // than leaning on the Next button's focus: the theme tiles are focusable
  // too, and once one is picked the button no longer holds focus.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        next();
      } else if (e.key === "ArrowLeft" && index > 0) {
        back();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [next, back, index]);

  const setTheme = (theme: ThemeChoice) => {
    applyTheme(theme);
    onChange({ theme });
  };

  return (
    <div className="flex flex-1 flex-col overflow-hidden">
      <div className="flex flex-1 items-center justify-center overflow-auto px-8 py-6">
        {/* Keyed on the step so React remounts the panel and the entry
            animation replays on every advance instead of only the first. */}
        <div key={step} className="w-full max-w-md animate-slide-up">
          {step === "welcome" ? (
            <div className="flex flex-col items-center text-center">
              <Mark className="size-16" />
              <h1 className="mt-6 text-2xl font-semibold tracking-tight text-balance text-heading">
                Welcome to Inertia
              </h1>
              <p className="mt-2 max-w-[22rem] text-sm leading-relaxed text-pretty text-muted-foreground">
                A native desktop shell with its own window chrome, a two-palette
                design system, and nothing you have to configure before it feels
                like yours. This takes about thirty seconds.
              </p>
            </div>
          ) : null}

          {step === "appearance" ? (
            <div>
              <StepHeading
                title="Pick your appearance"
                blurb="Changes apply immediately. System follows your OS and flips with it."
              />
              <div className="mt-6">
                <ThemePicker value={preferences.theme} onChange={setTheme} />
              </div>
            </div>
          ) : null}

          {step === "chrome" ? (
            <div>
              <StepHeading
                title="About the window"
                blurb="The title bar is drawn by the app, not the OS, so a few gestures are worth knowing."
              />
              <ul className="mt-6 flex flex-col gap-2">
                <Tip icon={<Square className="size-3.5" />} title="Drag anywhere on the bar">
                  The whole strip moves the window - not just the empty part.
                </Tip>
                <Tip icon={<Copy className="size-3.5" />} title="Double-click to maximize">
                  It grows into the monitor's work area instead of snapping, and
                  stops short of the taskbar.
                </Tip>
                <Tip icon={<Sparkles className="size-3.5" />} title="Corners stay rounded">
                  The compositor owns the corner radius, so it holds even where
                  the system default is turned off.
                </Tip>
              </ul>
            </div>
          ) : null}

          {step === "ready" ? (
            <div className="flex flex-col items-center text-center">
              <div className="accent-fill flex size-12 items-center justify-center rounded-full">
                <Check className="size-6 animate-pop-in" />
              </div>
              <h1 className="mt-5 text-2xl font-semibold tracking-tight text-balance text-heading">
                You're set
              </h1>
              <p className="mt-2 max-w-[22rem] text-sm leading-relaxed text-pretty text-muted-foreground">
                Everything here can be changed later. This walkthrough won't
                show again - delete <code className="font-mono text-xs">preferences.json</code> in
                the app's config folder to bring it back.
              </p>
            </div>
          ) : null}
        </div>
      </div>

      <footer className="flex items-center justify-between gap-4 px-8 pb-7 pt-1">
        <button
          type="button"
          onClick={back}
          aria-label="Back"
          className={cn(
            "inline-flex h-9 items-center gap-1.5 rounded-full px-3 text-sm text-muted-foreground",
            "outline-none transition-all duration-150 ease-out",
            "hover:fill-control-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
            // Hidden rather than unmounted, so the row's layout doesn't shift
            // between the first step and the rest.
            index === 0 && "pointer-events-none opacity-0",
          )}
        >
          <ArrowLeft className="size-3.5" />
          Back
        </button>

        <div className="flex items-center gap-1.5" aria-hidden="true">
          {STEPS.map((id, i) => (
            <span key={id} className={cn("step-dot", i === index && "step-dot-active")} />
          ))}
        </div>

        <button
          type="button"
          onClick={next}
          className={cn(
            "inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full accent-fill px-4",
            "text-sm font-medium outline-none transition-opacity duration-150 ease-out",
            "hover:opacity-80 active:opacity-90 focus-visible:ring-2 focus-visible:ring-ring",
          )}
        >
          {isLast ? "Start using Inertia" : "Continue"}
          {isLast ? <Check className="size-3.5" /> : <ArrowRight className="size-3.5" />}
        </button>
      </footer>
    </div>
  );
}

function StepHeading({ title, blurb }: { title: string; blurb: string }) {
  return (
    <div className="flex flex-col items-center text-center">
      {/* `text-balance` on the heading and `text-pretty` on the blurb stop the
          browser leaving one orphaned word on a second line, which is most of
          what made these read as ragged at this measure. */}
      <h1 className="text-2xl font-semibold tracking-tight text-balance text-heading">{title}</h1>
      <p className="mt-2 max-w-[22rem] text-sm leading-relaxed text-pretty text-muted-foreground">
        {blurb}
      </p>
    </div>
  );
}

function Tip({
  icon,
  title,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <li className="card-surface-subtle flex items-start gap-3 rounded-md p-3">
      <span className="mt-px flex size-7 shrink-0 items-center justify-center rounded-full fill-secondary text-foreground-secondary">
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium leading-snug text-foreground">{title}</span>
        <span className="mt-1 block text-xs leading-relaxed text-pretty text-muted-foreground">
          {children}
        </span>
      </span>
    </li>
  );
}
