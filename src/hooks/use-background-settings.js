import * as React from "react";

/**
 * The two switches that decide what Inertia does without a window.
 *
 * Not part of the preferences store, and deliberately so: one of these is a
 * Windows registry entry and the other is read by the main process before the
 * renderer exists. Both answers live outside the renderer, so the renderer asks
 * for them rather than keeping its own copy that could disagree with the
 * machine - a user who removed Inertia from startup in Task Manager must see an
 * off switch here the next time the pane opens.
 *
 * `available` is false in a browser tab and in tests, where there is no preload
 * bridge at all. The pane hides the rows rather than showing dead controls.
 */
export function useBackgroundSettings() {
  const api = typeof window === "undefined" ? null : window.backgroundAPI;
  const [state, setState] = React.useState(null);
  const [loading, setLoading] = React.useState(!!api);

  const reload = React.useCallback(async () => {
    if (!api) return;
    setLoading(true);
    try {
      setState(await api.get());
    } catch {
      // The main process not answering means the window is going away anyway.
      setState(null);
    } finally {
      setLoading(false);
    }
  }, [api]);

  React.useEffect(() => {
    reload();
  }, [reload]);

  /**
   * Flip a switch and take the main process's word for the result.
   *
   * Not optimistic, unlike most settings here. Setting a login item can be
   * refused by the OS or by policy, and a switch that slides across and then
   * silently means nothing is worse than one that takes a moment.
   */
  const set = React.useCallback(
    async (key, value) => {
      if (!api) return;
      const call = key === "launchAtLogin" ? api.setLaunchAtLogin : api.setMinimiseToTray;
      try {
        setState(await call(value));
      } catch {
        await reload();
      }
    },
    [api, reload]
  );

  return {
    available: !!api,
    loading,
    minimiseToTray: state?.minimiseToTray ?? true,
    launchAtLogin: !!state?.launchAtLogin,
    launchAtLoginSupported: !!state?.launchAtLoginSupported,
    platform: state?.platform ?? "",
    setMinimiseToTray: (value) => set("minimiseToTray", value),
    setLaunchAtLogin: (value) => set("launchAtLogin", value),
    reload,
  };
}
