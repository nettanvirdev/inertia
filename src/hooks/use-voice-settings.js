import * as React from "react";
import { useWorkspace } from "@/lib/workspace";
import { VOICE_DEFAULTS, VOICE_DOCUMENT } from "@/lib/voice";

/**
 * The voice settings, read from and written to the workspace.
 *
 * They live in the workspace folder rather than in app state for the reason
 * everything else does: copy the folder to another machine and the voice comes
 * with it. They are a document rather than a collection because there is
 * exactly one of them.
 *
 * Three screens read this - the pane that edits it, the composer that dictates
 * with it, and the thread that decides whether to read a reply aloud - so a
 * save writes the whole document and then hands every reader the same object
 * back, rather than each of them holding a copy that drifts.
 */
export function useVoiceSettings() {
  const { client, configured } = useWorkspace();
  const [settings, setSettings] = React.useState(VOICE_DEFAULTS);
  const [loading, setLoading] = React.useState(configured);

  const reload = React.useCallback(async () => {
    if (!configured) {
      setSettings(VOICE_DEFAULTS);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const stored = await client.readDocument(VOICE_DOCUMENT, {});
      setSettings({ ...VOICE_DEFAULTS, ...(stored && typeof stored === "object" ? stored : {}) });
    } catch {
      // A document that cannot be read is a document that has not been written
      // yet, which is the same thing as the defaults.
      setSettings(VOICE_DEFAULTS);
    } finally {
      setLoading(false);
    }
  }, [client, configured]);

  React.useEffect(() => {
    reload();
  }, [reload]);

  /**
   * Change one or more fields.
   *
   * Optimistic, because a settings control that waits for a disk write before
   * moving feels broken. The write is fire-and-forget for the same reason; a
   * failure here means the workspace folder is gone, which every other pane is
   * already shouting about.
   */
  const update = React.useCallback(
    (patch) => {
      setSettings((previous) => {
        const next = { ...previous, ...patch };
        if (configured) {
          client.writeDocument(VOICE_DOCUMENT, next).catch(() => {
            /* the workspace pane owns telling the user the folder is unreachable */
          });
        }
        return next;
      });
    },
    [client, configured]
  );

  return { settings, update, loading, reload };
}

/**
 * The microphones and speakers this machine actually has.
 *
 * The old pane listed "MacBook Pro Microphone" and "AirPods Pro" from a
 * hardcoded array, on every machine, including the Windows ones. Real devices
 * come from the browser, and their labels only exist after microphone access
 * has been granted once - before that the list is real but anonymous, so an
 * unlabelled device is shown by its position rather than by an empty string.
 */
export function useAudioDevices() {
  const [devices, setDevices] = React.useState({ inputs: [], outputs: [] });

  const read = React.useCallback(async () => {
    if (!navigator?.mediaDevices?.enumerateDevices) return;
    try {
      const all = await navigator.mediaDevices.enumerateDevices();
      const pick = (kind) =>
        all
          .filter((device) => device.kind === kind && device.deviceId !== "default")
          .map((device, index) => ({
            value: device.deviceId,
            label: device.label || `${kind === "audioinput" ? "Microphone" : "Output"} ${index + 1}`,
          }));
      setDevices({ inputs: pick("audioinput"), outputs: pick("audiooutput") });
    } catch {
      setDevices({ inputs: [], outputs: [] });
    }
  }, []);

  React.useEffect(() => {
    read();
    if (!navigator?.mediaDevices) return undefined;
    // Plugging in a headset while the pane is open should change the list.
    navigator.mediaDevices.addEventListener?.("devicechange", read);
    return () => navigator.mediaDevices.removeEventListener?.("devicechange", read);
  }, [read]);

  return { ...devices, refresh: read };
}
