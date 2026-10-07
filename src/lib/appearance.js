/**
 * Appearance, as one place that knows what the choices are.
 *
 * Every option here has to change something a person can see, or it does not
 * belong in this file. The settings screen used to be full of controls that
 * saved a value nothing read - a font size slider that moved no text, a density
 * toggle that tightened nothing. A preference that does not apply is worse than
 * a missing feature, because the user believes they configured it.
 *
 * So the mechanism is deliberately narrow: everything is stamped on the root
 * element, either as a CSS variable the stylesheet already reads or as a data
 * attribute a rule keys off. Nothing threads through React props, nothing needs
 * a component to opt in, and adding an option is one entry in DEFAULTS, one
 * line in `applyAppearance`, and - where the token is not one the stylesheet
 * already consumes - one rule in globals.css to read it.
 */

import { readPref, writePref } from "@/lib/persist";
import { DEFAULT_TOOL_ACCESS } from "@shared/tool-access";
import { DEFAULT_PROJECT_INIT } from "@shared/project-init";

/** The shipped values. `resetAppearance` and the reset button both use these. */
export const APPEARANCE_DEFAULTS = {
  accent: "default",
  // The ground, chosen per theme rather than once. Somebody who reads on a
  // warm cream page in daylight still wants a near-black one at night, and a
  // single "palette" setting would make them re-pick every time the theme
  // flipped.
  lightPalette: "default",
  darkPalette: "default",
  fontFamily: "sf",
  monoFamily: "sf",
  radius: "soft",
  density: "comfortable",
  zoom: 100,
  fontSize: 14,
  // 12.5 rather than a round number because that is the size the code block was
  // drawn at before it was a setting, and a default that quietly restyles every
  // existing transcript is not a default.
  codeSize: 12.5,
  lineHeight: "normal",
  messageWidth: "comfortable",
  // How tall an opened tool card may grow before it scrolls inside itself.
  // Without a ceiling, opening the card for a 300-line file pushed the rest of
  // the conversation off the screen and left the reader scrolling the whole
  // transcript to get back to where they were.
  toolHeight: 320,
  transcript: "bubbles",
  contrast: "normal",
  reduceMotion: false,
  showTimestamps: true,
  showAvatars: true,
  sidebarDefault: "expanded",
};

/**
 * Everything the settings sheet can set, appearance and behaviour together.
 *
 * This is what "Reset all settings" resets to, so it has to be the whole set:
 * a default missing from here is a preference the reset button silently leaves
 * alone, which is the kind of bug nobody reports and everybody hits once.
 */
export const PREFERENCE_DEFAULTS = {
  ...APPEARANCE_DEFAULTS,
  sendOnEnter: true,
  locale: "system",
  timezone: "system",
  // "last" means resume wherever the window was closed, which is what the app
  // has always done. Resetting settings should hand that behaviour back, not
  // pin the user to a screen they never chose.
  startupView: "last",
  defaultAgentId: null,
  defaultModelId: null,
  // What a new conversation starts as. The composer's two pills read from
  // the thread, and the thread is stamped with these when it is created, so
  // a person who always wants Autonomous and Never ask sets it once here
  // rather than twice per chat. Kept as the shipped defaults until changed.
  defaultMode: "chat",
  defaultApproval: "ask",
  // Whether a turn carries every tool it could ever use, or the ones it uses
  // constantly plus a way to ask for the rest. `all` is what the app has always
  // done; see shared/tool-access.js for what the other one trades.
  toolAccess: DEFAULT_TOOL_ACCESS,
  // Whether a conversation that is nearly out of window summarises itself once,
  // visibly, rather than letting the loop summarise a slightly different prefix
  // on every step from then on. On by default: both cost a summary, and only
  // one of them tells you it happened. `/autocompact off` in the composer.
  autoCompact: true,
  // Memory. On by default, because an assistant that forgets everything between
  // conversations is the thing people complain about most; every part of it can
  // be turned off, and turning the first one off turns off all of it.
  memory: true,
  memoryProject: true,
  memoryCapture: "session",
  memoryReview: false,
  memoryBudget: null,
  memoryBackend: "local",
  memoryInstructions: "",
  // Never write into somebody's repository without asking.
  projectInit: DEFAULT_PROJECT_INIT,
};

/**
 * Grounds.
 *
 * An accent tints a button; a palette changes the paper. The two are separate
 * settings because they answer different questions - "which colour is the app"
 * and "what am I reading on" - and a person who wants a warm page almost never
 * wants a warm button to go with it.
 *
 * `default` in both lists is the app's own palette, untouched: the tokens it
 * names are the ones in `:root` and `.dark`, tuned over a long time against
 * real contrast ratios, and the entry exists so choosing it removes the
 * attribute rather than overriding the file with a copy of itself. Every other
 * entry is an override block in globals.css that moves the ground and the
 * surfaces built on it, and nothing else - status colours, syntax, accents and
 * diagrams are the same in all of them, because "sepia" is a reading
 * preference and not a different design.
 *
 * `swatch` is what the settings tile paints itself, so the picker shows the
 * choice rather than describing it: ground, surface, and the ink on top.
 */
export const LIGHT_PALETTES = [
  {
    value: "default",
    label: "Default",
    hint: "The app's own: white ground, cool blue-grey surfaces.",
    swatch: { ground: "#ffffff", surface: "#eff3f8", ink: "#333537" },
  },
  {
    value: "paper",
    label: "Paper",
    hint: "Warm off-white. The calmest for long reading.",
    swatch: { ground: "#fbfaf7", surface: "#f2efe9", ink: "#33312e" },
  },
  {
    value: "sepia",
    label: "Sepia",
    hint: "Cream and brown ink. The least tiring over several hours.",
    swatch: { ground: "#f6efe2", surface: "#ece2ce", ink: "#3b3229" },
  },
  {
    value: "mist",
    label: "Mist",
    hint: "A soft grey ground with white surfaces. No large bright area to glare.",
    swatch: { ground: "#eef0f2", surface: "#ffffff", ink: "#2f3336" },
  },
  {
    value: "sky",
    label: "Sky",
    hint: "A cool pale blue. The ground a long document reads best on in daylight.",
    swatch: { ground: "#f2f6fb", surface: "#e8ecf2", ink: "#2c333d" },
    more: true,
  },
  {
    value: "sage",
    label: "Sage",
    hint: "Pale green-grey, the calmest of the cool ones.",
    swatch: { ground: "#f3f6f1", surface: "#e9ece7", ink: "#2f352d" },
    more: true,
  },
  {
    value: "linen",
    label: "Linen",
    hint: "Warm and pink rather than yellow. Sepia without the age.",
    swatch: { ground: "#faf6f2", surface: "#f0ece8", ink: "#382f2b" },
    more: true,
  },
];

export const DARK_PALETTES = [
  {
    value: "default",
    label: "Default",
    hint: "The app's own: neutral dark grey, the most legible for code.",
    swatch: { ground: "#181818", surface: "#232323", ink: "#e8e8e8" },
  },
  {
    value: "midnight",
    label: "Midnight",
    hint: "A blue-black ground. Deeper, without going to pure black.",
    swatch: { ground: "#12151c", surface: "#1b1f29", ink: "#e4e8f0" },
  },
  {
    value: "graphite",
    label: "Graphite",
    hint: "Warm and neutral, the blue taken out. The one for late at night.",
    swatch: { ground: "#1a1917", surface: "#242320", ink: "#e9e6e1" },
  },
  {
    value: "ink",
    label: "Ink",
    hint: "Near-black, for an OLED panel or a dark room.",
    swatch: { ground: "#0a0a0a", surface: "#161616", ink: "#e6e6e6" },
  },
  {
    value: "ocean",
    label: "Ocean",
    hint: "A deep blue-green. Midnight with the cold taken out.",
    swatch: { ground: "#0e1a1f", surface: "#182429", ink: "#dbe9ee" },
    more: true,
  },
  {
    value: "forest",
    label: "Forest",
    hint: "Dark green-grey. The quietest of the dark grounds.",
    swatch: { ground: "#111a15", surface: "#1b241f", ink: "#dee9e1" },
    more: true,
  },
  {
    value: "plum",
    label: "Plum",
    hint: "A deep violet-black, for a dark room.",
    swatch: { ground: "#17121d", surface: "#211c27", ink: "#e7e1ee" },
    more: true,
  },
];

/**
 * Accents.
 *
 * "Default" is the quiet-surface original: the accent IS the foreground, which
 * is why the app has never had a colour in it. Every other entry is a single
 * hue with a foreground chosen for contrast against it, and nothing else about
 * the design changes - no gradient, no shadow, no coloured surfaces.
 */
export const ACCENTS = [
  { value: "default", label: "Ink", light: null, dark: null },
  {
    value: "emerald",
    label: "Emerald",
    light: "#047857",
    dark: "#34d399",
    on: { light: "#ffffff", dark: "#06251b" },
  },
  {
    value: "blue",
    label: "Blue",
    light: "#1d4ed8",
    dark: "#60a5fa",
    on: { light: "#ffffff", dark: "#0a1834" },
  },
  {
    value: "violet",
    label: "Violet",
    light: "#6d28d9",
    dark: "#a78bfa",
    on: { light: "#ffffff", dark: "#1b0f36" },
  },
  {
    value: "amber",
    label: "Amber",
    light: "#b45309",
    dark: "#fbbf24",
    on: { light: "#ffffff", dark: "#2b1a00" },
  },
  {
    value: "rose",
    label: "Rose",
    light: "#be123c",
    dark: "#fb7185",
    on: { light: "#ffffff", dark: "#37060f" },
  },
  {
    value: "cyan",
    label: "Cyan",
    light: "#0e7490",
    dark: "#22d3ee",
    on: { light: "#ffffff", dark: "#04252c" },
  },
  // Past here is what the "More colours" button reveals. The split is not a
  // ranking - every one of these is the same three values as the seven above -
  // it is a row count. Sixteen swatches open by default is a paint chart, and
  // the person who wanted the app to be blue has to read past nine colours to
  // find out that Ink means no colour at all.
  {
    value: "teal",
    label: "Teal",
    light: "#0f766e",
    dark: "#2dd4bf",
    on: { light: "#ffffff", dark: "#03231f" },
    more: true,
  },
  {
    value: "sky",
    label: "Sky",
    light: "#0369a1",
    dark: "#38bdf8",
    on: { light: "#ffffff", dark: "#042536" },
    more: true,
  },
  {
    value: "indigo",
    label: "Indigo",
    light: "#4338ca",
    dark: "#818cf8",
    on: { light: "#ffffff", dark: "#131238" },
    more: true,
  },
  {
    value: "fuchsia",
    label: "Fuchsia",
    light: "#a21caf",
    dark: "#e879f9",
    on: { light: "#ffffff", dark: "#2c0733" },
    more: true,
  },
  {
    value: "pink",
    label: "Pink",
    light: "#be185d",
    dark: "#f472b6",
    on: { light: "#ffffff", dark: "#330717" },
    more: true,
  },
  {
    value: "red",
    label: "Red",
    light: "#b91c1c",
    dark: "#f87171",
    on: { light: "#ffffff", dark: "#300a0a" },
    more: true,
  },
  {
    value: "orange",
    label: "Orange",
    light: "#c2410c",
    dark: "#fb923c",
    on: { light: "#ffffff", dark: "#2b1203" },
    more: true,
  },
  {
    value: "lime",
    label: "Lime",
    light: "#4d7c0f",
    dark: "#a3e635",
    on: { light: "#ffffff", dark: "#16240a" },
    more: true,
  },
  {
    value: "slate",
    label: "Slate",
    light: "#334155",
    dark: "#94a3b8",
    on: { light: "#ffffff", dark: "#0d1420" },
    more: true,
  },
];

/**
 * The ones a picker shows before anybody asks for more.
 *
 * `more: true` is the only difference between an entry here and one below the
 * fold - same shape, same three values, same everything. A stored preference
 * naming a hidden entry is still honoured and the picker opens expanded, so
 * nothing can be chosen and then become unreachable.
 */
export function shownFirst(options) {
  return options.filter((option) => !option.more);
}

/** Whether `value` is one of the entries behind the fold. */
export function isBehindMore(options, value) {
  return options.some((option) => option.more && option.value === value);
}

/**
 * The Bengali half of every stack.
 *
 * A font stack is consulted per character, not per string, so a Latin face with
 * no Bengali glyphs does not fail - it falls through to whatever the platform
 * offers, and the platform's answer changes by machine. That is why বাংলা used
 * to look like a different font depending on which *Latin* face was selected,
 * which is the one thing a typography setting must never do.
 *
 * So every stack names its Bengali partner explicitly, chosen to sit with the
 * Latin face rather than merely to exist: a grotesque with Noto Sans Bengali, a
 * serif with Noto Serif Bengali. `Nirmala UI` and `Kalpurush` stay at the end
 * for the case the bundled files have not loaded yet - a first paint in the
 * platform's own Bengali is better than a first paint in boxes.
 */
const BENGALI_SANS = '"Noto Sans Bengali Variable", "Noto Sans Bengali", "Nirmala UI", Kalpurush';
const BENGALI_SERIF = '"Noto Serif Bengali Variable", "Noto Serif Bengali", Vrinda, "Nirmala UI"';
const BENGALI_MONO = `${BENGALI_SANS}`;

/**
 * Fonts.
 *
 * Latin faces are still mostly system stacks - the ones a machine already has
 * cost nothing and render instantly. The exceptions are the three that are
 * bundled in `main.jsx`: Inter, Source Serif and JetBrains Mono were names in a
 * stack that did something only if you happened to have the font installed, and
 * an option that works on one machine and silently does nothing on the next is
 * not an option.
 *
 * Every entry ends in a Bengali face. See `BENGALI_SANS` above for why that is
 * not optional.
 */
export const FONT_FAMILIES = [
  {
    value: "sf",
    label: "SF Pro",
    description: "Apple's interface face, then the platform's own",
    stack: `"SF Pro Text", "SF Pro Display", -apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", "Inter Variable", Inter, system-ui, Roboto, "Helvetica Neue", Arial, ${BENGALI_SANS}, sans-serif`,
  },
  {
    value: "system",
    label: "System",
    description: "Whatever this computer uses",
    stack: `ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, ${BENGALI_SANS}, sans-serif`,
  },
  {
    value: "inter",
    label: "Inter",
    description: "The web's workhorse, bundled",
    stack: `"Inter Variable", Inter, "Inter var", "SF Pro Text", "Segoe UI", system-ui, ${BENGALI_SANS}, sans-serif`,
  },
  {
    value: "grotesk",
    label: "Grotesk",
    description: "Tighter, more neutral",
    stack: `"Segoe UI", "Inter Variable", Inter, Roboto, "Helvetica Neue", Arial, ${BENGALI_SANS}, sans-serif`,
  },
  {
    value: "serif",
    label: "Serif",
    description: "For long reading, in both scripts",
    stack: `"Source Serif 4 Variable", "Source Serif 4", Georgia, "Iowan Old Style", "Times New Roman", ${BENGALI_SERIF}, serif`,
  },
  {
    value: "mono",
    label: "Monospace",
    description: "Everything at one width",
    stack: `ui-monospace, "JetBrains Mono Variable", "Cascadia Mono", "SF Mono", Consolas, "Liberation Mono", ${BENGALI_MONO}, monospace`,
  },
];

/**
 * Monospace faces.
 *
 * A separate choice from the interface typeface, because the two are read for
 * different reasons: the UI font is picked for how a sentence looks and the
 * code font for whether a zero can be told from an O at eleven pixels. Picking
 * "Monospace" above used to be the only way to get a different code face, and
 * it took the whole interface with it.
 */
export const MONO_FAMILIES = [
  {
    value: "sf",
    label: "SF Mono",
    description: "Apple's code face, then the platform's own",
    stack: `"SF Mono", ui-monospace, "JetBrains Mono Variable", "JetBrains Mono", "Cascadia Code", "Cascadia Mono", Menlo, Consolas, "Liberation Mono", ${BENGALI_MONO}, monospace`,
  },
  {
    value: "jetbrains",
    label: "JetBrains Mono",
    description: "Tall x-height, drawn for code. Bundled.",
    stack: `"JetBrains Mono Variable", "JetBrains Mono", "Cascadia Code", ui-monospace, Consolas, ${BENGALI_MONO}, monospace`,
  },
  {
    value: "system",
    label: "System",
    description: "The platform's own",
    stack: `ui-monospace, "Cascadia Mono", "SF Mono", Consolas, "Liberation Mono", ${BENGALI_MONO}, monospace`,
  },
  {
    value: "consolas",
    label: "Consolas",
    description: "Narrow, high contrast",
    stack: `Consolas, "Cascadia Mono", ui-monospace, ${BENGALI_MONO}, monospace`,
  },
  {
    value: "courier",
    label: "Courier",
    description: "Typewriter, wide",
    stack: `"Courier New", Courier, ui-monospace, ${BENGALI_MONO}, monospace`,
  },
  {
    value: "dejavu",
    label: "DejaVu",
    description: "Round, generous spacing",
    stack: `"DejaVu Sans Mono", "Liberation Mono", "Menlo", ui-monospace, ${BENGALI_MONO}, monospace`,
  },
];

/**
 * Transcript style.
 *
 * "Flat" is not a second layout - it is the same markup with the tint taken off
 * the user's message, so a long conversation reads as one document rather than
 * as a chat log. Doing it with the colour token rather than a class means the
 * transcript components never learn that this setting exists.
 */
export const TRANSCRIPTS = [
  { value: "bubbles", label: "Bubbles", description: "Your messages sit in a tinted bubble" },
  { value: "flat", label: "Flat", description: "No tint, one continuous document" },
];

/** Line spacing inside a message. Numbers, so they multiply the chosen size. */
export const LINE_HEIGHTS = [
  { value: "tight", label: "Tight", ratio: 1.45 },
  { value: "normal", label: "Normal", ratio: 1.625 },
  { value: "relaxed", label: "Relaxed", ratio: 1.85 },
];

/**
 * Secondary-text contrast.
 *
 * The design leans hard on muted grey for anything that is not the sentence you
 * are reading, which is calm on a good panel and unreadable on a dim laptop in
 * daylight. High contrast pulls that grey back toward the foreground rather
 * than replacing the palette, so nothing else about the theme changes.
 */
export const CONTRASTS = [
  { value: "normal", label: "Normal" },
  { value: "high", label: "High" },
];

/** Corner radius. The token every rounded thing in the app derives from. */
export const RADII = [
  { value: "sharp", label: "Sharp", rem: 0.375 },
  { value: "soft", label: "Soft", rem: 1 },
  { value: "round", label: "Round", rem: 1.5 },
];

export const DENSITIES = [
  { value: "comfortable", label: "Comfortable" },
  { value: "compact", label: "Compact" },
];

/** How wide a transcript line is allowed to run. */
export const MESSAGE_WIDTHS = [
  { value: "narrow", label: "Narrow", ch: "58ch" },
  { value: "comfortable", label: "Comfortable", ch: "72ch" },
  { value: "wide", label: "Wide", ch: "92ch" },
  { value: "full", label: "Full width", ch: "100%" },
];

const byValue = (list, value, fallback) =>
  list.find((entry) => entry.value === value) ?? list.find((entry) => entry.value === fallback);

/**
 * The same lookup, falling back to what the app actually ships.
 *
 * Every call site used to name its own fallback as a string - `"system"` for
 * both typefaces - which is a second copy of the default that nothing keeps in
 * step. Changing the shipped font would have left the fallback pointing at the
 * old one, and a workspace written by a newer build would land somewhere the
 * defaults no longer mention.
 */
const shipped = (list, key, value) => byValue(list, value, APPEARANCE_DEFAULTS[key]);

/** Read appearance out of the preferences bag, filling anything missing. */
export function appearanceOf(preferences) {
  return { ...APPEARANCE_DEFAULTS, ...(preferences ?? {}) };
}

/**
 * Put the choices on the document.
 *
 * Called on every change and once at startup. It is idempotent and cheap: a
 * handful of `setProperty` calls, no layout read, so it can run in an effect
 * without a guard.
 *
 * `isDark` has to be passed in rather than sniffed, because the accent needs
 * the RESOLVED theme - "system" is not a value you can pick a colour for.
 */
/** Stamps a palette, or takes the attribute off entirely for the default one. */
function setPalette(root, key, value) {
  if (value === "default") delete root.dataset[key];
  else root.dataset[key] = value;
}

export function applyAppearance(preferences, isDark) {
  if (typeof document === "undefined") return;

  const a = appearanceOf(preferences);
  const root = document.documentElement;
  const style = root.style;

  const accent = byValue(ACCENTS, a.accent, "default");
  if (accent?.light) {
    style.setProperty("--accent-solid", isDark ? accent.dark : accent.light);
    style.setProperty("--accent-on", isDark ? accent.on.dark : accent.on.light);
  } else {
    // Back to the original: the accent is the ink, which is what the app looked
    // like before any of this existed.
    style.removeProperty("--accent-solid");
    style.removeProperty("--accent-on");
  }

  style.setProperty("--font-sans", shipped(FONT_FAMILIES, "fontFamily", a.fontFamily).stack);
  style.setProperty("--font-mono", shipped(MONO_FAMILIES, "monoFamily", a.monoFamily).stack);
  style.setProperty("--radius", `${byValue(RADII, a.radius, "soft").rem}rem`);
  style.setProperty("--chat-font-size", `${clamp(a.fontSize, 12, 20)}px`);
  style.setProperty("--code-font-size", `${clamp(a.codeSize, 11, 18)}px`);
  style.setProperty("--tool-max-height", `${clamp(a.toolHeight, 160, 800)}px`);
  style.setProperty(
    "--chat-line-height",
    String(byValue(LINE_HEIGHTS, a.lineHeight, "normal").ratio)
  );
  style.setProperty("--chat-measure", byValue(MESSAGE_WIDTHS, a.messageWidth, "comfortable").ch);

  // Both of these are read by an attribute selector in globals.css rather than
  // by a component, which is what lets them restyle the transcript without the
  // transcript importing anything from here.
  root.dataset.transcript = byValue(TRANSCRIPTS, a.transcript, "bubbles").value;
  root.dataset.contrast = byValue(CONTRASTS, a.contrast, "normal").value;

  /*
   * The grounds. Both are stamped whatever the current theme is, because the
   * rules that read them are already scoped by `.dark` - stamping only the
   * active one would leave the other attribute behind from the last time the
   * theme flipped, and a stale attribute is a palette nobody chose.
   *
   * "default" removes the attribute rather than setting it. There is no
   * `[data-light-palette="default"]` block in the stylesheet and there must not
   * be: the default palette is `:root` itself, and writing it twice is how the
   * copy and the original start to disagree.
   */
  setPalette(root, "lightPalette", byValue(LIGHT_PALETTES, a.lightPalette, "default").value);
  setPalette(root, "darkPalette", byValue(DARK_PALETTES, a.darkPalette, "default").value);

  // Tailwind v4 derives its whole spacing scale from `--spacing`, so density is
  // one number rather than a class every component has to remember to read.
  const compact = a.density === "compact";
  // 3px rather than the 0.215rem (3.44px) this used to be: the control spec is
  // a 2px grid, and a base that lands on a fraction of a pixel puts every step
  // built from it on one too.
  style.setProperty("--spacing", compact ? "0.1875rem" : "0.25rem");
  root.dataset.density = compact ? "compact" : "comfortable";
  // A string, not a boolean: `dataset` stringifies either way, and `"false"` is
  // truthy in a CSS attribute selector, so the attribute has to be absent.
  if (a.reduceMotion) root.dataset.reduceMotion = "true";
  else delete root.dataset.reduceMotion;

  setZoom(clamp(a.zoom, 70, 150) / 100);
  writeMirror(a);
}

/**
 * The mirror, and why there is one.
 *
 * Appearance is saved in the workspace folder so it travels with the files, and
 * the workspace is read over IPC well after the window has already painted. For
 * the first second of every launch the app therefore knew nothing about the
 * user's accent, typeface or density, and rendered the shipped defaults before
 * snapping to their settings - a flash of somebody else's application.
 *
 * So `applyAppearance` also drops a copy in localStorage, which is synchronous
 * and readable before React exists. The workspace stays the source of truth;
 * this is only ever a guess at what the folder is about to say, and it is
 * overwritten the moment the real answer arrives.
 */
const MIRROR_KEY = "appearance";

function writeMirror(applied) {
  const mirror = {};
  for (const key of Object.keys(APPEARANCE_DEFAULTS)) mirror[key] = applied[key];
  writePref(MIRROR_KEY, mirror);
}

/**
 * Paint the remembered appearance before React mounts.
 *
 * Called from `main.jsx` next to `bootstrapTheme`, and for the same reason it
 * is a function rather than an inline `<script>`: the window's CSP is
 * `script-src 'self'`, so nothing inline runs at all.
 *
 * It has to run AFTER `bootstrapTheme`, because the accent has a light and a
 * dark value and the only way to know which one to paint is to read the theme
 * that has just been applied to the document.
 */
export function bootstrapAppearance() {
  if (typeof document === "undefined") return;
  const isDark = document.documentElement.classList.contains("dark");
  applyAppearance(readPref(MIRROR_KEY, null), isDark);
}

/**
 * Window zoom.
 *
 * The webview's own zoom rather than a root font-size, because the app sizes text
 * in pixels in a hundred places and rem scaling would move about a third of it.
 * Zoom moves everything, including the parts nobody remembered to make relative.
 */
function setZoom(factor) {
  const api = typeof window !== "undefined" ? window.electronAPI : null;
  if (typeof api?.setZoom === "function") api.setZoom(factor);
}

function clamp(value, min, max) {
  const n = Number(value);
  if (!Number.isFinite(n)) return min;
  return Math.min(max, Math.max(min, n));
}
