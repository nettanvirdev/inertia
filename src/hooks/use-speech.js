import * as React from "react";
import { speechFor } from "@shared/speech";
import { speaker } from "@/lib/speaker";
import { useVoiceSettings } from "./use-voice-settings";

/**
 * Speaking, from anywhere.
 *
 * Three screens want the same two things - what is being spoken right now, and
 * a way to speak a reply - and each of them wanting its own copy is how you end
 * up with two voices at once. Both come off the one shared speaker.
 */

const IDLE = { status: "idle", caption: "", id: null };

/** What the shared speaker is doing. Re-renders when that changes. */
export function useSpeechState() {
  return React.useSyncExternalStore(
    (notify) => speaker.subscribe(notify),
    () => speaker.state,
    () => IDLE
  );
}

/**
 * Speak a reply, in the voice the settings chose.
 *
 * The markdown is never sent as written: `speechFor` names a code block rather
 * than reading it, flattens tables, drops emoji and link URLs, and splits the
 * rest into real sentences so speech can start after the first one.
 */
export function useSpeakMarkdown() {
  const { settings } = useVoiceSettings();
  const current = React.useRef(settings);
  current.current = settings;

  const speak = React.useCallback(async (id, markdown) => {
    const { voiceId, modelId, rate } = current.current;
    if (!voiceId) throw new Error("No voice is chosen yet. Pick one under Settings, Voice.");
    const lines = speechFor(markdown ?? "");
    if (!lines.length) return;
    await speaker.speak(lines, { voiceId, modelId, rate, id });
  }, []);

  const toggle = React.useCallback(
    async (id, markdown) => {
      // Pressing the button on the reply that is already talking means stop.
      if (speaker.state.id === id && speaker.state.status !== "idle") {
        speaker.stop();
        return;
      }
      await speak(id, markdown);
    },
    [speak]
  );

  return { speak, toggle, stop: () => speaker.stop() };
}
