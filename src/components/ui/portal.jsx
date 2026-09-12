import * as React from "react";
import { createPortal } from "react-dom";

export const useIsomorphicLayoutEffect =
  typeof document !== "undefined" ? React.useLayoutEffect : React.useEffect;

/** One shared body-level container per id, created once and never torn down. */
function getContainer(id) {
  if (typeof document === "undefined") return null;
  let el = document.getElementById(id);
  if (!el) {
    el = document.createElement("div");
    el.id = id;
    // No positioning/z-index here: a stacking context on the container would
    // trap every layer inside it and break the 50/70/80 ladder.
    document.body.appendChild(el);
  }
  return el;
}

export function Portal({ children, containerId = "inertia-portal" }) {
  const [container] = React.useState(() => getContainer(containerId));
  if (!container) return null;
  return createPortal(children, container);
}

/* ── Escape ladder ──────────────────────────────────────────────────────────
 * Every floating layer registers here instead of owning a bare document
 * listener. A single capture-phase listener fires ONLY the topmost layer and
 * stops the event dead, so Escape inside a select that lives in a dialog closes
 * the select and leaves the dialog standing. (It also pre-empts the Escape
 * branch of `useDismiss`, which is why layers never double-close.)
 * ─────────────────────────────────────────────────────────────────────────── */

const escapeStack = [];
let escapeListening = false;

function onDocumentEscape(e) {
  if (e.key !== "Escape") return;
  const top = escapeStack[escapeStack.length - 1];
  if (!top) return;
  e.preventDefault();
  e.stopPropagation();
  top.onEscape(e);
}

function startEscapeListener() {
  if (escapeListening || typeof document === "undefined") return;
  document.addEventListener("keydown", onDocumentEscape, true);
  escapeListening = true;
}

function stopEscapeListener() {
  if (!escapeListening || escapeStack.length) return;
  document.removeEventListener("keydown", onDocumentEscape, true);
  escapeListening = false;
}

/** Register `onEscape` as the topmost dismissable layer while `open`. */
export function useEscapeLayer(open, onEscape) {
  const handler = React.useRef(onEscape);
  handler.current = onEscape;

  React.useEffect(() => {
    if (!open) return;
    const entry = { onEscape: (e) => handler.current?.(e) };
    escapeStack.push(entry);
    startEscapeListener();
    return () => {
      const i = escapeStack.indexOf(entry);
      if (i !== -1) escapeStack.splice(i, 1);
      stopEscapeListener();
    };
  }, [open]);
}

/** True while the user has asked the OS to reduce motion. */
export function usePrefersReducedMotion() {
  const [reduced, setReduced] = React.useState(false);

  React.useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReduced(mq.matches);
    const onChange = (e) => setReduced(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  return reduced;
}
