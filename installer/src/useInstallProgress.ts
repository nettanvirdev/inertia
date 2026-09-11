import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

export type Phase = "idle" | "preparing" | "installing" | "finishing" | "done";

type ProgressEvent = { phase: Phase; message: string };

/**
 * A silent NSIS install reports nothing while it runs - no file count, no byte
 * total, nothing to read a percentage out of. So the bar is honest about being
 * phase-based: each phase owns a ceiling, and the displayed value eases toward
 * that ceiling rather than jumping to it. During the long copy it approaches
 * asymptotically, which reads as "still working" without ever claiming to know
 * how much is left. Only "done" is allowed to reach 100.
 */
const CEILINGS: Record<Phase, number> = {
  idle: 0,
  preparing: 12,
  installing: 86,
  finishing: 94,
  done: 100,
};

/** How hard the displayed value is pulled toward its ceiling, per frame. */
const PULL: Record<Phase, number> = {
  idle: 0.2,
  preparing: 0.08,
  // Slow on purpose: this phase lasts seconds, and a fast pull would park the
  // bar at 86% almost immediately and then sit there looking stuck.
  installing: 0.004,
  finishing: 0.1,
  done: 0.25,
};

export function useInstallProgress() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [message, setMessage] = useState("");
  const [percent, setPercent] = useState(0);

  const phaseRef = useRef<Phase>("idle");
  phaseRef.current = phase;

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;

    void listen<ProgressEvent>("install://progress", (event) => {
      setPhase(event.payload.phase);
      setMessage(event.payload.message);
    }).then((off) => {
      if (disposed) off();
      else unlisten = off;
    });

    let frame = 0;
    const step = () => {
      const current = phaseRef.current;
      setPercent((value) => {
        const ceiling = CEILINGS[current];
        const next = value + (ceiling - value) * PULL[current];
        // Land exactly on 100 instead of asymptotically near it, so the bar
        // fills rather than stopping a hairline short.
        return ceiling - next < 0.4 ? ceiling : next;
      });
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);

    return () => {
      disposed = true;
      unlisten?.();
      cancelAnimationFrame(frame);
    };
  }, []);

  const reset = () => {
    setPhase("idle");
    setMessage("");
    setPercent(0);
  };

  return { phase, message, percent, reset };
}
