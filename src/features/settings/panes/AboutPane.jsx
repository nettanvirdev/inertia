import * as React from "react";
import { cn } from "@/lib/utils";
import { SettingsCard, SettingsSection } from "../SettingsRow";

const DASH = " - ";

export function AboutPane() {
  const [info, setInfo] = React.useState(null);

  // In a browser (vite dev with no desktop shell) there is no bridge - every
  // row falls back to a dash rather than rendering "undefined".
  React.useEffect(() => {
    let alive = true;
    Promise.resolve(window.electronAPI?.getAppInfo?.())
      .then((result) => {
        if (alive && result) setInfo(result);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  /**
   * What this build is made of.
   *
   * The rows named Electron, Node and Chromium before, and they were dashes:
   * this shell is Tauri, so the pane was asking for three versions that do not
   * exist and leaving the three that do off the list. A pane whose only job is
   * to say what is installed has to name the right things.
   *
   * The webview row takes its label from the backend rather than hardcoding
   * "WebView2", because the same field is WKWebView on macOS and WebKitGTK on
   * Linux. `platform` and `arch` are one row - "windows · x86_64" is how
   * somebody reads it, and two rows would only make the pane taller.
   */
  const platform = [info?.platform, info?.arch].filter(Boolean).join(" · ");
  const rows = [
    { label: "App version", value: info?.version ?? info?.appVersion },
    { label: "Tauri", value: info?.tauri },
    { label: "Rust", value: info?.rust },
    { label: info?.webviewName ?? "WebView", value: info?.webview },
    { label: "Platform", value: platform },
    // Only in a development build. In a shipped one the row would say
    // "release" to everybody, forever, which is a row that tells nobody
    // anything - but seeing "development" is worth a line when a bug report
    // turns out to be about a debug build.
    ...(info?.profile === "development"
      ? [{ label: "Build", value: "development" }]
      : []),
  ];

  return (
    <div className="w-full">
      <div className="flex items-center gap-3">
        <img
          src="./assets/logo-256.png"
          alt=""
          className="size-14 shrink-0 rounded-lg object-contain"
        />
        <div className="min-w-0">
          <p className="text-base font-medium text-foreground">Inertia</p>
          <p className="text-xs text-muted-foreground">
            Version {rows[0].value ?? DASH}
          </p>
          <p className="mt-1 text-[0.6875rem] leading-relaxed text-muted-foreground">
            Persistent AI teammates with their own computers, memory and
            routines.
          </p>
        </div>
      </div>

      <SettingsSection flat title="Runtime">
        <SettingsCard className="flex flex-col gap-0.5 py-1.5">
          {rows.map((row) => (
            <div
              key={row.label}
              className="flex items-baseline justify-between gap-4 px-1 py-1"
            >
              <span className="text-xs text-foreground/90">{row.label}</span>
              <span
                className={cn(
                  "shrink-0 font-mono text-[0.6875rem] tabular-nums",
                  row.value
                    ? "text-muted-foreground"
                    : "text-muted-foreground/60",
                )}
              >
                {row.value ?? DASH}
              </span>
            </div>
          ))}
        </SettingsCard>
      </SettingsSection>
    </div>
  );
}
