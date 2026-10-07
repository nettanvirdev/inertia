import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";

const STORAGE_KEY = "inertia.theme";
const ThemeContext = createContext(null);

/**
 * The theme a fresh install opens in.
 *
 * Exported rather than written twice, because the other place that needs it is
 * the "reset to shipped defaults" button - and a reset that puts a preference
 * somewhere the app does not actually ship is the exact bug that pane's own
 * comment warns about.
 */
export const DEFAULT_THEME = "dark";

/** Resolve "system" against the OS preference. */
function systemTheme() {
  if (typeof window === "undefined") return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/**
 * The user's choice, or dark.
 *
 * Dark rather than "system" is deliberate: the app is designed dark-first, the
 * installer that precedes it is dark unconditionally, and following the OS
 * meant a machine set to light opened Inertia in a theme nobody had chosen and
 * that the palette is not tuned for. "system" remains a setting - it is just no
 * longer the answer given on behalf of someone who has not answered.
 *
 * Only the fallback changes. A stored choice of any kind still wins, so this
 * cannot override somebody who has already picked light or system.
 */
function readStored() {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "dark" || v === "light" || v === "system") return v;
  } catch {}
  return DEFAULT_THEME;
}

/**
 * Paint the stored theme before React mounts, so a light-theme user never sees
 * the dark default flash. Called from main.jsx rather than an inline <script>
 * because the window's CSP is `script-src 'self'` - no inline execution.
 */
export function bootstrapTheme() {
  const choice = readStored();
  const dark = choice === "dark" || (choice === "system" && systemTheme() === "dark");
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.style.colorScheme = dark ? "dark" : "light";
}

/**
 * Class-strategy theme provider. `theme` is the user's choice
 * ("system" | "light" | "dark"); `resolved` is what is actually painted.
 * Transitions are suppressed during a switch - every token animating at
 * once reads as a bug.
 */
export function ThemeProvider({ children }) {
  const [theme, setThemeState] = useState(readStored);
  const [resolved, setResolved] = useState(() =>
    readStored() === "system" ? systemTheme() : readStored()
  );

  useEffect(() => {
    const next = theme === "system" ? systemTheme() : theme;
    setResolved(next);

    const root = document.documentElement;
    const style = document.createElement("style");
    style.appendChild(
      document.createTextNode("*,*::before,*::after{transition:none !important}")
    );
    document.head.appendChild(style);

    root.classList.toggle("dark", next === "dark");
    root.style.colorScheme = next;

    // force a reflow, then re-enable transitions
    void window.getComputedStyle(document.body).opacity;
    requestAnimationFrame(() => style.remove());
  }, [theme]);

  useEffect(() => {
    if (theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setThemeState("system");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [theme]);

  /**
   * Persisted here rather than in the effect above, so that only a choice is
   * written down.
   *
   * The effect runs on mount as well as on change, so it stored whatever the
   * fallback happened to be - which turned "has not chosen" into "chose this"
   * the first time the app ever painted. Nobody noticed while the fallback and
   * the shipped default were the same value; the moment they differed, every
   * existing install was pinned to the old one and could not be moved except by
   * clearing site data.
   */
  const setTheme = useCallback((next) => {
    setThemeState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // A session that cannot persist still gets to change its own theme.
    }
  }, []);

  const value = useMemo(
    () => ({
      theme,
      resolved,
      setTheme,
      toggle: () => setTheme(resolved === "dark" ? "light" : "dark"),
    }),
    [theme, resolved, setTheme]
  );

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme() {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used inside <ThemeProvider>");
  return ctx;
}
