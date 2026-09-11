import { invoke } from "@tauri-apps/api/core";

export type ThemeChoice = "system" | "light" | "dark";

export type Preferences = {
  onboardingCompleted: boolean;
  theme: ThemeChoice;
};

export const DEFAULT_PREFERENCES: Preferences = {
  onboardingCompleted: false,
  theme: "dark",
};

export async function loadPreferences(): Promise<Preferences> {
  try {
    return await invoke<Preferences>("load_preferences");
  } catch {
    // Running outside Tauri (a plain `vite dev` in a browser) has no backing
    // store. Defaults mean the walkthrough shows, which is the useful state
    // to land in while designing it.
    return DEFAULT_PREFERENCES;
  }
}

export async function savePreferences(preferences: Preferences): Promise<void> {
  try {
    await invoke("save_preferences", { preferences });
  } catch {
    // A preference that fails to persist is not worth failing a click over -
    // the in-memory value still drives this session.
  }
}

const media = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;

/**
 * Paints a theme choice onto `<html>`. "system" is resolved live rather than
 * frozen at the moment of choosing, so the app follows the OS when it flips.
 */
export function applyTheme(theme: ThemeChoice) {
  const dark = theme === "dark" || (theme === "system" && !!media?.matches);
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.classList.toggle("light", !dark);
}

/** Re-applies on OS changes while the choice is "system". Returns a disposer. */
export function watchSystemTheme(getTheme: () => ThemeChoice): () => void {
  if (!media) return () => {};
  const onChange = () => {
    if (getTheme() === "system") applyTheme("system");
  };
  media.addEventListener("change", onChange);
  return () => media.removeEventListener("change", onChange);
}
