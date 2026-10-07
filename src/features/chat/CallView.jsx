import * as React from "react";
import { speechFor } from "@shared/speech";
import { cn } from "@/lib/utils";
import { speaker } from "@/lib/speaker";
import { useDictation } from "@/hooks/use-dictation";
import { useSpeechState } from "@/hooks/use-speech";
import { useVoiceSettings } from "@/hooks/use-voice-settings";
import { ChevronDown } from "@/components/icons";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { IconButton } from "@/components/ui/icon-button";
import { Portal } from "@/components/ui/portal";
import { Progress } from "@/components/ui/progress";
import { usePresence } from "@/hooks/use-presence";
import { useSwap } from "@/features/chat/arrival";

/**
 * A call: talking to an agent with no keyboard in it.
 *
 * Half duplex on purpose. The microphone is closed while the agent is speaking,
 * because an open one would hear the agent through the speakers and answer
 * itself, and the fix for that is echo cancellation good enough to be its own
 * project. So the loop is strictly one at a time: listen, work, speak, listen.
 *
 * There is no transcript on screen and no scrollback. Everything said here goes
 * into the thread underneath, which is the real record; this is a surface for
 * the thirty seconds you are holding a coffee, and reading is what the thread
 * is for. What it does show is the sentence being spoken right now, because
 * following a long answer by ear is much easier with one line of anchor.
 *
 * Silence is the send key: the dictation hook calibrates itself to the room and
 * stops recording once you stop talking, so a call needs no press-and-hold and
 * no push-to-talk binding. Send now exists because that calibration is still a
 * guess about a room, and the first version of this screen had no way to submit
 * when the guess was wrong. It simply listened forever.
 */

const PHASE_WORDS = {
  listening: "Listening",
  thinking: "Working",
  speaking: "Speaking",
};

export function CallView({
  agent,
  messages,
  streaming,
  minimized = false,
  onSend,
  onMinimize,
  onClose,
}) {
  const { settings } = useVoiceSettings();
  const speech = useSpeechState();

  const [phase, setPhase] = React.useState("listening");
  const [error, setError] = React.useState(null);

  // Replies that existed when the call opened, so it narrates the conversation
  // from here rather than reading the backlog at whoever just picked up.
  const spoken = React.useRef(null);
  if (spoken.current === null) {
    spoken.current = new Set((messages ?? []).map((message) => message.id));
  }

  const closing = React.useRef(false);
  const phaseRef = React.useRef(phase);
  phaseRef.current = phase;

  const dictation = useDictation({
    deviceId: settings.inputDeviceId,
    language: settings.language,
    onText: (text) => {
      if (closing.current) return;
      setError(null);
      setPhase("thinking");
      onSend?.(text);
    },
    onError: (failure) => {
      if (closing.current) return;
      setError(failure.message);
      setPhase("listening");
    },
  });

  // The key handler is bound once and must not be rebound on every render just
  // to see the current recorder.
  const dictationRef = React.useRef(dictation);
  dictationRef.current = dictation;

  const listen = React.useCallback(() => {
    if (closing.current) return;
    setPhase("listening");
    dictation.start();
  }, [dictation]);

  const listenRef = React.useRef(listen);
  listenRef.current = listen;

  /**
   * Opening the call opens the microphone.
   *
   * This effect used to guard itself with a `started` ref so it would only fire
   * once. That is wrong in a way that only shows up in development: StrictMode
   * mounts every component twice, refs survive the remount, and so the
   * microphone was opened on the throwaway first mount, torn down with it, and
   * never opened again on the real one. The screen then sat saying "Listening"
   * over a device nobody had turned on, forever.
   *
   * Starting in the effect and stopping in its cleanup is the shape that
   * survives being run twice, which is the whole point of StrictMode running
   * it twice.
   */
  React.useEffect(() => {
    closing.current = false;
    listenRef.current();
    return () => {
      closing.current = true;
      dictationRef.current?.cancel();
      speaker.stop();
    };
  }, []);

  const hangUp = React.useCallback(() => {
    closing.current = true;
    dictation.cancel();
    speaker.stop();
    onClose?.();
  }, [dictation, onClose]);

  /** Cut the agent off, or throw away what it just heard, and listen again. */
  const interrupt = React.useCallback(() => {
    if (phaseRef.current === "speaking") speaker.stop();
    else dictation.cancel();
    setError(null);
    listen();
  }, [dictation, listen]);

  /* -- narrate the newest finished reply ---------------------------------- */
  React.useEffect(() => {
    if (closing.current || streaming) return;
    const fresh = (messages ?? []).filter(
      (message) =>
        message.role === "agent" && message.status === "sent" && !spoken.current.has(message.id)
    );
    for (const message of fresh) spoken.current.add(message.id);

    const latest = fresh[fresh.length - 1];
    if (!latest) return;

    const lines = speechFor(latest.content ?? "");
    if (!lines.length) {
      // A reply that is all tool calls has nothing to say out loud, and waiting
      // for speech that will never start is how a call deadlocks.
      listen();
      return;
    }
    if (!settings.voiceId) {
      setError("No voice is chosen yet. Pick one under Settings, Voice.");
      listen();
      return;
    }

    setPhase("speaking");
    speaker
      .speak(lines, {
        voiceId: settings.voiceId,
        modelId: settings.modelId,
        rate: settings.rate,
        id: latest.id,
      })
      .then(() => {
        if (!closing.current) listen();
      })
      .catch((failure) => {
        if (closing.current) return;
        setError(failure.message);
        listen();
      });
  }, [messages, streaming, settings.voiceId, settings.modelId, settings.rate, listen]);

  /* -- the two keys ------------------------------------------------------- */
  React.useEffect(() => {
    // A backgrounded call must not eat Space and Enter: the composer has focus
    // down there, and a person typing a message is not trying to interrupt.
    if (minimized) return undefined;
    const onKey = (event) => {
      if (event.key === "Escape") {
        event.preventDefault();
        hangUp();
        return;
      }
      // Enter sends, because that is the key this app already sends with, and
      // there is no text field here for it to type a newline into.
      if (event.key === "Enter" && phaseRef.current === "listening") {
        event.preventDefault();
        dictationRef.current?.stop();
        return;
      }
      // Space is the interrupt because it is the key a hand is already near.
      if (event.key === " " && phaseRef.current !== "listening") {
        event.preventDefault();
        interrupt();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hangUp, interrupt, minimized]);

  const caption =
    phase === "listening"
      ? dictation.state === "thinking"
        ? "Working out what you said…"
        : "Say something. Stop talking and it sends, or press Send now."
      : phase === "speaking"
        ? speech.caption || "…"
        : "Thinking about it.";

  /**
   * Backgrounded: no card, but the call is still live.
   *
   * Every hook above this line keeps running - the microphone stays open, the
   * loop keeps narrating replies - because the call is a conversation, not a
   * dialog, and putting it behind the thread should not end it any more than
   * looking away from a phone hangs it up. Only the sheet goes - and it goes
   * the way it came, held for a beat so the exit can be seen.
   */
  const sheet = usePresence(!minimized);
  // The word and the button change with the phase; each fades to the next
  // rather than being swapped under the eye.
  const word = useSwap(phase);
  const action = useSwap(phase === "listening");

  if (!sheet.mounted) return null;

  return (
    <Portal>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`Call with ${agent?.name ?? "your agent"}`}
        className="fixed inset-0 z-50 grid place-items-center px-5"
      >
        {/* The same scrim and the same sheet every other overlay in the app
            uses. The first version reached for `fill-whisper`, which is a two
            percent tint meant to sit on top of an opaque surface, not to be
            one: the card was see-through and the transcript read straight
            through the middle of it. */}
        <div
          data-state={sheet.state}
          className="absolute inset-0 scrim animate-fade-in"
          aria-hidden="true"
        />
        <div
          data-state={sheet.state}
          className="relative w-full max-w-[26rem] overlay-surface rounded-3xl p-6 text-center animate-overlay-in"
        >
          <div className="flex items-center justify-between">
            <p className="text-[0.6875rem] uppercase tracking-wider text-muted-foreground">Call</p>
            <IconButton size="sm" label="Send the call to the background" onClick={onMinimize}>
              <ChevronDown />
            </IconButton>
          </div>
          <p className="mt-1 text-lg text-foreground">{agent?.name ?? "Agent"}</p>

          <p className="mt-3 flex items-center justify-center gap-2 text-[13px] text-foreground/90">
            <span
              aria-hidden="true"
              className={cn(
                "size-1.5 rounded-full transition-colors duration-[var(--motion-base)] ease-[var(--ease-out)]",
                phase === "listening" && "bg-success animate-soft-pulse",
                phase === "thinking" && "bg-warning animate-soft-pulse",
                phase === "speaking" && "bg-info"
              )}
            />
            <span
              key={phase}
              onAnimationEnd={word.onAnimationEnd}
              className={cn(word.swapping && "animate-fade-in")}
            >
              {PHASE_WORDS[phase]}
            </span>
          </p>

          {/* The meter is the one thing that answers "is it hearing me", which
              is the question every voice interface gets asked first. */}
          <Collapse open={phase === "listening" && dictation.state === "listening"}>
            <Progress
              className="mx-auto mt-3 w-32"
              value={dictation.level}
              max={1}
              label="Microphone level"
            />
          </Collapse>

          <p className="mt-3 min-h-[3.2em] text-[13px] leading-relaxed text-muted-foreground">
            {caption}
          </p>

          <Collapse open={Boolean(error)}>
            <p className="mt-1 text-[0.6875rem] leading-relaxed text-destructive-ink">{error}</p>
          </Collapse>

          <div className="mt-5 flex items-center justify-center gap-2">
            {/* Silence is meant to send, and usually does. But an endpoint
                detector is a guess about a room, and a call whose only other
                buttons throw work away is a dead end the moment that guess is
                wrong. This is the way out that always works. */}
            {phase === "listening" ? (
              <Button
                key="send"
                size="pill"
                variant="primary"
                disabled={dictation.state !== "listening"}
                onClick={() => dictation.stop()}
                onAnimationEnd={action.onAnimationEnd}
                className={cn(action.swapping && "animate-fade-in")}
              >
                Send now
              </Button>
            ) : (
              <Button
                key="interrupt"
                size="pill"
                variant="secondary"
                onClick={interrupt}
                onAnimationEnd={action.onAnimationEnd}
                className={cn(action.swapping && "animate-fade-in")}
              >
                Interrupt
              </Button>
            )}
            <Button size="pill" variant="danger" onClick={hangUp}>
              Hang up
            </Button>
          </div>

          <p className="mt-4 text-[0.6875rem] text-muted-foreground">
            {phase === "listening"
              ? "Enter sends · Esc hangs up"
              : "Space interrupts · Esc hangs up"}
          </p>
          <p className="mt-1 text-[0.6875rem] text-muted-foreground/80">
            Send it to the background and it keeps going while you read.
          </p>
        </div>
      </div>
    </Portal>
  );
}
