/**
 * Dressing xterm.js in the app's colours.
 *
 * xterm.js paints to a canvas, so it cannot inherit anything: every colour has
 * to be handed to it as a concrete value, and it will not re-read them when
 * the theme changes. This reads the app's own CSS variables off the document
 * and turns them into the object xterm wants, which is why the pane calls it
 * again whenever the theme moves.
 *
 * The sixteen ANSI colours are NOT taken from the app's palette, and that is
 * deliberate. They are not decoration - they are a protocol. A program writing
 * "colour 1" means red, and a build that prints its failures in the app's
 * accent colour because red happened to map to it is a build whose failures do
 * not look like failures. The app's tokens set the ground, the text and the
 * cursor; the sixteen stay recognisably themselves, tuned for contrast against
 * a light or a dark ground rather than replaced.
 */

/** Read one CSS custom property as a concrete colour, or fall back. */
function readVar(styles, name, fallback) {
  const value = styles.getPropertyValue(name)?.trim();
  return value || fallback;
}

/**
 * The ANSI sixteen, twice.
 *
 * Two sets rather than one adjusted programmatically: the adjustment that
 * makes a palette readable on a light ground is not a formula, and a
 * generated one produces a blue nobody can read on white every time.
 */
const DARK_ANSI = {
  black: "#1c1c1c",
  red: "#f26d6d",
  green: "#7bd88f",
  yellow: "#e8c76a",
  blue: "#7aa6f5",
  magenta: "#c792ea",
  cyan: "#5fd7d7",
  white: "#d6d6d6",
  brightBlack: "#6b6b6b",
  brightRed: "#ff8a8a",
  brightGreen: "#95e6a8",
  brightYellow: "#f5d98a",
  brightBlue: "#9ec1ff",
  brightMagenta: "#dcb0f5",
  brightCyan: "#84e8e8",
  brightWhite: "#f5f5f5",
};

const LIGHT_ANSI = {
  black: "#2a2a2a",
  red: "#c0392b",
  green: "#237a3d",
  yellow: "#8a6100",
  blue: "#1f5fbf",
  magenta: "#8e44ad",
  cyan: "#0f7b7b",
  white: "#4a4a4a",
  brightBlack: "#767676",
  brightRed: "#d64535",
  brightGreen: "#2c9450",
  brightYellow: "#a37400",
  brightBlue: "#2b74d9",
  brightMagenta: "#a259c4",
  brightCyan: "#128f8f",
  brightWhite: "#1a1a1a",
};

/**
 * Whether the ground this is drawn on is dark.
 *
 * Measured from the colour the app actually resolved rather than from a theme
 * name, because the app has three theme states - light, dark, and following
 * the system - and only the resolved colour knows what the third one came out
 * as. Anything unparseable reads as dark, which is the app's own default.
 */
function isDark(background) {
  const rgb = /rgba?\(\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)/i.exec(background);
  const hex = /^#([0-9a-f]{6})$/i.exec(background?.trim() ?? "");
  let r;
  let g;
  let b;
  if (rgb) {
    [, r, g, b] = rgb.map(Number);
  } else if (hex) {
    const n = parseInt(hex[1], 16);
    r = (n >> 16) & 255;
    g = (n >> 8) & 255;
    b = n & 255;
  } else {
    return true;
  }
  // Perceived luminance. The weights are the usual ones and the threshold is
  // the middle; nothing here needs to be more precise than "which half".
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255 < 0.5;
}

/** The theme object xterm.js takes, read from the app as it is right now. */
export function xtermTheme(element) {
  const target = element ?? (typeof document !== "undefined" ? document.body : null);
  if (!target || typeof window === "undefined") return { ...DARK_ANSI, background: "#1c1c1c" };

  const styles = window.getComputedStyle(target);
  const background = readVar(styles, "--color-background", styles.backgroundColor || "#1c1c1c");
  const foreground = readVar(styles, "--color-foreground", styles.color || "#d6d6d6");
  const dark = isDark(background);

  return {
    ...(dark ? DARK_ANSI : LIGHT_ANSI),
    background,
    foreground,
    cursor: foreground,
    cursorAccent: background,
    // A selection that is a solid block hides the text under it. xterm draws
    // this one behind the glyphs, so it has to be translucent or the selected
    // command becomes unreadable at the moment you are trying to copy it.
    selectionBackground: dark ? "rgba(255,255,255,0.22)" : "rgba(0,0,0,0.16)",
  };
}

export { DARK_ANSI, LIGHT_ANSI, isDark };
