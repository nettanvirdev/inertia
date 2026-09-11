import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { SetupTitlebar } from "@setup/components/SetupTitlebar";
import { useInstallProgress } from "@setup/useInstallProgress";
import { Mark } from "@/onboarding/Mark";
import { Alert, ArrowRight, Check, Folder, Loader } from "@/lib/icons";
import { cn } from "@/lib/utils";

type SetupInfo = { productName: string; version: string; defaultDir: string };
type Screen = "ready" | "installing" | "done" | "error";

/**
 * An absolute Windows path does not fit the 230-odd pixels this row gives it,
 * and plain `truncate` would cut off the tail - which is the only part that
 * distinguishes one choice from another. So elide the middle instead, keeping
 * the drive and the last two segments. Done here rather than with the usual
 * `direction: rtl` trick: that leaves the browser's bidi algorithm to decide
 * where a path's colons and backslashes belong, and it does not always decide
 * the way a path needs. The full string stays in the `title`.
 */
function shortenPath(path: string, max = 38) {
  if (path.length <= max) return path;
  const parts = path.split("\\").filter(Boolean);
  if (parts.length <= 3) return path;
  return [parts[0], "…", ...parts.slice(-2)].join("\\");
}

export function Setup() {
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [dir, setDir] = useState("");
  const [shortcuts, setShortcuts] = useState(true);
  const [screen, setScreen] = useState<Screen>("ready");
  const [error, setError] = useState("");
  const { phase, message, percent, reset } = useInstallProgress();

  useEffect(() => {
    void invoke<SetupInfo>("setup_info")
      .catch((): SetupInfo => {
        // `bun run dev:setup-ui` opens this in a plain browser, where there is
        // no Rust side to answer. Standing in a plausible answer means the
        // setup screen can be designed and restyled at Vite's speed instead
        // of rebuilding the bootstrapper for every nudge. Nothing past this
        // screen works there, which is the honest limit of the shortcut.
        const placeholder = "C:\\Users\\you\\AppData\\Local\\inertia";
        return { productName: "inertia", version: "0.0.0-dev", defaultDir: placeholder };
      })
      .then((loaded) => {
        setInfo(loaded);
        setDir(loaded.defaultDir);
      });
  }, []);

  const change = useCallback(async () => {
    const picked = await open({ directory: true, multiple: false, defaultPath: dir });
    if (typeof picked !== "string") return;
    setDir(await invoke<string>("resolve_install_dir", { picked }));
  }, [dir]);

  const install = useCallback(async () => {
    setScreen("installing");
    setError("");
    try {
      await invoke("run_install", { dir, shortcuts });
      setScreen("done");
    } catch (e) {
      setError(typeof e === "string" ? e : "The installation did not finish.");
      setScreen("error");
    }
  }, [dir, shortcuts]);

  const busy = screen === "installing";

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <SetupTitlebar busy={busy} />

      <div className="flex flex-1 flex-col overflow-hidden px-8 pb-7 pt-2">
        {/* Keyed so each screen plays its entry animation on arrival. */}
        <div key={screen} className="flex flex-1 animate-slide-up flex-col">
          {screen === "ready" && info ? (
            <Ready
              info={info}
              dir={dir}
              shortcuts={shortcuts}
              onChangeDir={() => void change()}
              onToggleShortcuts={() => setShortcuts((v) => !v)}
              onInstall={() => void install()}
            />
          ) : null}

          {screen === "installing" ? (
            <Installing percent={percent} phase={phase} message={message} />
          ) : null}

          {screen === "done" && info ? <Done info={info} dir={dir} /> : null}

          {screen === "error" ? (
            <Failed
              error={error}
              onRetry={() => {
                reset();
                setScreen("ready");
              }}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}

function Ready({
  info,
  dir,
  shortcuts,
  onChangeDir,
  onToggleShortcuts,
  onInstall,
}: {
  info: SetupInfo;
  dir: string;
  shortcuts: boolean;
  onChangeDir: () => void;
  onToggleShortcuts: () => void;
  onInstall: () => void;
}) {
  return (
    <>
      <div className="flex flex-1 flex-col items-center justify-center text-center">
        <Mark className="size-16" />
        <h1 className="mt-5 text-2xl font-semibold tracking-tight text-heading">
          Install {info.productName}
        </h1>
        <p className="mt-1.5 text-xs font-medium text-muted-foreground">Version {info.version}</p>
        <p className="mt-3 max-w-[19rem] text-sm leading-relaxed text-muted-foreground">
          This installs for your account only, so Windows won't ask for
          administrator access.
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <div className="card-surface-subtle flex items-center gap-2.5 rounded-md p-2.5">
          <span className="flex size-7 shrink-0 items-center justify-center rounded-full fill-secondary text-foreground-secondary">
            <Folder className="size-3.5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
              Location
            </span>
            <span
              title={dir}
              className="block truncate text-left text-xs text-foreground"
            >
              {shortenPath(dir)}
            </span>
          </span>
          <button
            type="button"
            onClick={onChangeDir}
            className="shrink-0 rounded-full px-2.5 py-1 text-xs font-medium text-foreground-secondary outline-none transition-colors duration-150 ease-out hover:fill-control-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
          >
            Change
          </button>
        </div>

        <Checkbox checked={shortcuts} onChange={onToggleShortcuts}>
          Add shortcuts to the Start menu and Desktop
        </Checkbox>

        <button
          type="button"
          onClick={onInstall}
          className="mt-2 inline-flex h-10 items-center justify-center gap-1.5 rounded-full accent-fill text-sm font-medium outline-none transition-opacity duration-150 ease-out hover:opacity-80 active:opacity-90 focus-visible:ring-2 focus-visible:ring-ring"
        >
          Install
          <ArrowRight className="size-3.5" />
        </button>
      </div>
    </>
  );
}

function Installing({
  percent,
  phase,
  message,
}: {
  percent: number;
  phase: string;
  message: string;
}) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center text-center">
      <Loader className="size-10 animate-spin-slow text-foreground-secondary" />
      <h1 className="mt-6 text-xl font-semibold tracking-tight text-heading">Installing</h1>
      <p className="mt-1.5 h-5 text-sm text-muted-foreground">{message}</p>

      <div className="mt-6 w-full max-w-[17rem]">
        <div
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(percent)}
          aria-label={`Installing: ${phase}`}
          className="h-1.5 w-full overflow-hidden rounded-full fill-track"
        >
          {/* Width is driven per frame by the easing in `useInstallProgress`,
              so there is no CSS transition here - two easings stacked on the
              same value fight each other and the bar stutters. */}
          <div
            className="h-full rounded-full bg-foreground"
            style={{ width: `${percent}%` }}
          />
        </div>
        <p className="mt-2 text-right font-mono text-[11px] tabular-nums text-muted-foreground">
          {Math.round(percent)}%
        </p>
      </div>
    </div>
  );
}

function Done({ info, dir }: { info: SetupInfo; dir: string }) {
  return (
    <>
      <div className="flex flex-1 flex-col items-center justify-center text-center">
        <div className="accent-fill flex size-12 items-center justify-center rounded-full">
          <Check className="size-6 animate-pop-in" />
        </div>
        <h1 className="mt-5 text-2xl font-semibold tracking-tight text-heading">
          {info.productName} is installed
        </h1>
        <p className="mt-2 max-w-[19rem] text-sm leading-relaxed text-muted-foreground">
          It's in <span className="text-foreground-secondary">{dir}</span>, and you can remove it
          any time from Installed apps.
        </p>
      </div>

      <button
        type="button"
        onClick={() => void invoke("launch_app", { dir })}
        className="inline-flex h-10 items-center justify-center gap-1.5 rounded-full accent-fill text-sm font-medium outline-none transition-opacity duration-150 ease-out hover:opacity-80 active:opacity-90 focus-visible:ring-2 focus-visible:ring-ring"
      >
        Launch {info.productName}
        <ArrowRight className="size-3.5" />
      </button>
    </>
  );
}

function Failed({ error, onRetry }: { error: string; onRetry: () => void }) {
  return (
    <>
      <div className="flex flex-1 flex-col items-center justify-center text-center">
        <div className="flex size-12 items-center justify-center rounded-full bg-destructive-wash text-destructive-ink">
          <Alert className="size-6" />
        </div>
        <h1 className="mt-5 text-xl font-semibold tracking-tight text-heading">
          That didn't finish
        </h1>
        <p className="mt-2 max-w-[19rem] text-sm leading-relaxed text-muted-foreground">{error}</p>
      </div>

      <button
        type="button"
        onClick={onRetry}
        className="inline-flex h-10 items-center justify-center rounded-full accent-fill text-sm font-medium outline-none transition-opacity duration-150 ease-out hover:opacity-80 active:opacity-90 focus-visible:ring-2 focus-visible:ring-ring"
      >
        Try again
      </button>
    </>
  );
}

function Checkbox({
  checked,
  onChange,
  children,
}: {
  checked: boolean;
  onChange: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      onClick={onChange}
      className="flex items-center gap-2.5 rounded-md p-2.5 text-left outline-none transition-colors duration-150 ease-out hover:fill-whisper focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span
        className={cn(
          "flex size-4 shrink-0 items-center justify-center rounded-xs transition-colors duration-150 ease-out",
          checked ? "accent-fill" : "fill-field ring-1 ring-control-border",
        )}
      >
        {checked ? <Check className="size-2.5 animate-pop-in" /> : null}
      </span>
      <span className="text-xs text-foreground-secondary">{children}</span>
    </button>
  );
}
