import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SetupTitlebar } from "@setup/components/SetupTitlebar";
import { useInstallProgress } from "@setup/useInstallProgress";
import { Mark } from "@setup/components/Mark";
import { TriangleAlert as Alert, Check, Loader, X } from "@/components/icons";
import { cn } from "@/lib/utils";

type UninstallInfo = { productName: string; version: string; installDir: string };
type Screen = "confirm" | "working" | "done" | "error";

export function Uninstall() {
  const [info, setInfo] = useState<UninstallInfo | null>(null);
  const [wipeSettings, setWipeSettings] = useState(false);
  const [screen, setScreen] = useState<Screen>("confirm");
  const [error, setError] = useState("");
  const { message, percent } = useInstallProgress();

  useEffect(() => {
    void invoke<UninstallInfo>("uninstall_info")
      .catch((): UninstallInfo => ({
        productName: "inertia",
        version: "0.0.0-dev",
        installDir: "C:\\Users\\you\\AppData\\Local\\inertia",
      }))
      .then(setInfo);
  }, []);

  const remove = useCallback(async () => {
    setScreen("working");
    setError("");
    try {
      await invoke("run_uninstall", { wipeSettings });
      setScreen("done");
    } catch (e) {
      setError(typeof e === "string" ? e : "The uninstall did not finish.");
      setScreen("error");
    }
  }, [wipeSettings]);

  const busy = screen === "working";

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <SetupTitlebar busy={busy} title="Uninstall" />

      <div className="flex flex-1 flex-col overflow-hidden px-8 pb-7 pt-2">
        <div key={screen} className="flex flex-1 animate-slide-up flex-col">
          {screen === "confirm" && info ? (
            <>
              <div className="flex flex-1 flex-col items-center justify-center text-center">
                {/* The mark is dimmed rather than replaced with a warning
                    glyph: this is a deliberate, reversible action, not an
                    error, and a red icon here would overstate it. */}
                <Mark className="size-16 opacity-45" />
                <h1 className="mt-5 text-2xl font-semibold tracking-tight text-balance text-heading">
                  Remove {info.productName}?
                </h1>
                <p className="mt-2 max-w-[19rem] text-sm leading-relaxed text-pretty text-muted-foreground">
                  This deletes the app and its shortcuts. You can install it
                  again at any time.
                </p>
              </div>

              <div className="flex flex-col gap-2">
                <Choice
                  checked={wipeSettings}
                  onChange={() => setWipeSettings((v) => !v)}
                  title="Also delete my settings"
                  blurb="Your theme and the first-run walkthrough state. Leave this off to keep them for next time."
                />

                <button
                  type="button"
                  onClick={() => void remove()}
                  className="mt-2 inline-flex h-10 items-center justify-center gap-1.5 rounded-full bg-destructive text-sm font-medium text-destructive-foreground outline-none transition-opacity duration-150 ease-out hover:opacity-85 active:opacity-95 focus-visible:ring-2 focus-visible:ring-ring"
                >
                  Uninstall
                </button>
              </div>
            </>
          ) : null}

          {screen === "working" ? (
            <div className="flex flex-1 flex-col items-center justify-center text-center">
              <Loader className="size-10 animate-spin-slow text-foreground-secondary" />
              <h1 className="mt-6 text-xl font-semibold tracking-tight text-heading">Removing</h1>
              <p className="mt-1.5 h-5 text-sm text-muted-foreground">{message}</p>
              <div className="mt-6 w-full max-w-[17rem]">
                <div
                  role="progressbar"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(percent)}
                  className="h-1.5 w-full overflow-hidden rounded-full fill-track"
                >
                  <div className="h-full rounded-full bg-foreground" style={{ width: `${percent}%` }} />
                </div>
              </div>
            </div>
          ) : null}

          {screen === "done" && info ? (
            <>
              <div className="flex flex-1 flex-col items-center justify-center text-center">
                <div className="accent-fill flex size-12 items-center justify-center rounded-full">
                  <Check className="size-6 animate-pop-in" />
                </div>
                <h1 className="mt-5 text-2xl font-semibold tracking-tight text-balance text-heading">
                  {info.productName} is gone
                </h1>
                <p className="mt-2 max-w-[19rem] text-sm leading-relaxed text-pretty text-muted-foreground">
                  Thanks for trying it. Sorry to see you go.
                </p>
              </div>
              <button
                type="button"
                onClick={() => void invoke("close_uninstaller")}
                className="inline-flex h-10 items-center justify-center rounded-full accent-fill text-sm font-medium outline-none transition-opacity duration-150 ease-out hover:opacity-80 active:opacity-90 focus-visible:ring-2 focus-visible:ring-ring"
              >
                Close
              </button>
            </>
          ) : null}

          {screen === "error" ? (
            <>
              <div className="flex flex-1 flex-col items-center justify-center text-center">
                <div className="flex size-12 items-center justify-center rounded-full bg-destructive-wash text-destructive-ink">
                  <Alert className="size-6" />
                </div>
                <h1 className="mt-5 text-xl font-semibold tracking-tight text-heading">
                  That didn't finish
                </h1>
                <p className="mt-2 max-w-[19rem] text-sm leading-relaxed text-pretty text-muted-foreground">
                  {error}
                </p>
              </div>
              <button
                type="button"
                onClick={() => setScreen("confirm")}
                className="inline-flex h-10 items-center justify-center rounded-full accent-fill text-sm font-medium outline-none transition-opacity duration-150 ease-out hover:opacity-80 active:opacity-90 focus-visible:ring-2 focus-visible:ring-ring"
              >
                Try again
              </button>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}

/** A checkbox with a second line of explanation - this one deletes data. */
function Choice({
  checked,
  onChange,
  title,
  blurb,
}: {
  checked: boolean;
  onChange: () => void;
  title: string;
  blurb: string;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      onClick={onChange}
      className={cn(
        "flex items-start gap-2.5 rounded-md p-3 text-left outline-none",
        "transition-colors duration-150 ease-out focus-visible:ring-2 focus-visible:ring-ring",
        checked ? "card-surface-raised" : "card-surface-subtle hover:fill-control-hover",
      )}
    >
      <span
        className={cn(
          "mt-px flex size-4 shrink-0 items-center justify-center rounded-xs transition-colors duration-150 ease-out",
          checked ? "bg-destructive text-destructive-foreground" : "fill-field ring-1 ring-control-border",
        )}
      >
        {checked ? <X className="size-2.5 animate-pop-in" /> : null}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-xs font-medium text-foreground">{title}</span>
        <span className="mt-1 block text-[11px] leading-relaxed text-pretty text-muted-foreground">
          {blurb}
        </span>
      </span>
    </button>
  );
}
