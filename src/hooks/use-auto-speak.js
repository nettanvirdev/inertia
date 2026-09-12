import * as React from "react";
import { speechFor } from "@shared/speech";
import { speaker } from "@/lib/speaker";
import { useVoiceSettings } from "./use-voice-settings";

/**
 * Reading a reply out loud, once it is finished.
 *
 * Deliberately not as it streams. Speaking a partial sentence means either
 * synthesising the same words twice as more arrive, or guessing where a
 * sentence ends from text that has not finished being written - and both spend
 * the character quota on something a person then hears stutter. A reply that is
 * done is a reply we can split properly.
 *
 * Only ever the newest finished reply. Opening a thread with fifty messages in
 * it must not start reading the history, so the first render of a thread only
 * records where it came in and speaks nothing.
 *
 * Suspended while a call is open, because the call is already narrating the
 * same replies and two readings of one answer is worse than none.
 */
export function useAutoSpeak(messages, threadId, { suspended = false } = {}) {
  const { settings } = useVoiceSettings();

  const spoken = React.useRef(new Set());
  const knownThread = React.useRef(null);

  // Settings are read inside an effect that must not re-run when they change,
  // or toggling the switch would speak the last reply again.
  const current = React.useRef({ settings, suspended });
  current.current = { settings, suspended };

  React.useEffect(() => {
    // A different thread is a different conversation. Whatever was being said
    // about the last one is no longer wanted.
    if (knownThread.current !== threadId) {
      knownThread.current = threadId;
      speaker.stop();
      spoken.current = new Set(
        (messages ?? []).filter((m) => m.status !== "streaming").map((m) => m.id)
      );
      return;
    }

    const { settings: voice, suspended: paused } = current.current;
    if (paused || !voice.autoSpeak || !voice.voiceId) {
      // Still mark them seen, so switching the setting on mid-thread starts
      // with the next reply rather than with the backlog.
      for (const message of messages ?? []) {
        if (message.status !== "streaming") spoken.current.add(message.id);
      }
      return;
    }

    const finished = (messages ?? []).filter(
      (message) =>
        message.role === "agent" &&
        message.status === "sent" &&
        !message.stopped &&
        !spoken.current.has(message.id)
    );
    for (const message of finished) spoken.current.add(message.id);

    const latest = finished[finished.length - 1];
    if (!latest?.content?.trim()) return;

    const lines = speechFor(latest.content);
    if (!lines.length) return;

    speaker
      .speak(lines, {
        voiceId: voice.voiceId,
        modelId: voice.modelId,
        rate: voice.rate,
        id: latest.id,
      })
      .catch(() => {
        // A failure here is a reply that was read silently, which is what would
        // have happened anyway. The settings pane is where a broken key is
        // worth shouting about.
      });
  }, [messages, threadId]);
}
