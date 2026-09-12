import { raw } from "./envelope";

/**
 * The two switches that decide what Inertia does without a window.
 *
 * Raw answers rather than the `{ ok, data }` envelope most of this folder
 * speaks, because `hooks/use-background-settings.js` was written against
 * Electron's `ipcRenderer.invoke` and reads the settings object straight off
 * the call - `state?.minimiseToTray`, `!!state?.launchAtLoginSupported`. It
 * already wraps every call in a try/catch, so a throw is a shape it handles and
 * an envelope is not.
 *
 * Every method answers the whole settings object rather than an acknowledgement,
 * and that is the point of the pair of setters: setting a login item can be
 * refused by the OS or by policy, so the pane takes the app's word for what
 * happened rather than sliding a switch across optimistically and meaning
 * nothing by it.
 *
 * The shape is `{ minimiseToTray, launchAtLogin, launchAtLoginSupported,
 * platform }`, and `platform` is Node's spelling ("win32", "darwin", "linux")
 * because the General pane compares it against "darwin" to decide whether a
 * tray row makes sense at all.
 */
export function backgroundBridge() {
  return {
    get: () => raw("background_settings"),
    setMinimiseToTray: (value) => raw("background_set_minimise_to_tray", { value: !!value }),
    setLaunchAtLogin: (value) => raw("background_set_launch_at_login", { value: !!value }),
  };
}
