import { useEffect, useRef } from "react";

/**
 * Global hotkey. `combo` looks like "mod+k", "mod+shift+p", "escape".
 * `mod` is Cmd on macOS and Ctrl elsewhere.
 */
export function useHotkey(combo, handler, enabled = true) {
  useEffect(() => {
    if (!enabled) return;
    const parsed = parseCombo(combo);
    const onKeyDown = (e) => {
      if (!matches(parsed, e)) return;
      e.preventDefault();
      handler(e);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [combo, handler, enabled]);
}

/**
 * The same thing for a whole catalogue: `map` is combo string to handler.
 *
 * It exists because the shortcut list is data now, and a hook cannot be called
 * in a loop over data that changes length. One listener reads the current map
 * out of a ref, so adding or removing a shortcut costs nothing and re-renders
 * do not tear the window listener down and put it back every time.
 */
export function useHotkeys(map, enabled = true) {
  const latest = useRef(map);
  latest.current = map;

  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (e) => {
      for (const [combo, handler] of Object.entries(latest.current)) {
        if (!handler) continue;
        if (!matches(parseCombo(combo), e)) continue;
        e.preventDefault();
        handler(e);
        return;
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled]);
}

function parseCombo(combo) {
  const parts = String(combo).toLowerCase().split("+");
  return {
    key: parts[parts.length - 1],
    mod: parts.includes("mod"),
    shift: parts.includes("shift"),
    alt: parts.includes("alt"),
  };
}

/**
 * A field with a caret in it owns its plain keystrokes. Only combos carrying a
 * modifier are allowed to fire while someone is typing, so a future unmodified
 * shortcut cannot start eating letters out of the message composer.
 */
function isEditable(target) {
  if (!target || typeof target !== "object") return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return target.isContentEditable === true;
}

function matches(parsed, e) {
  const mod = e.metaKey || e.ctrlKey;
  if (parsed.mod !== mod) return false;
  if (parsed.shift !== e.shiftKey) return false;
  if (parsed.alt !== e.altKey) return false;
  if (String(e.key).toLowerCase() !== parsed.key) return false;
  if (!parsed.mod && !parsed.alt && isEditable(e.target)) return false;
  return true;
}
