/**
 * Turning a reply into something worth hearing.
 *
 * An agent answers in markdown - fences, tables, links, headings, bullets, file
 * paths - because that is what reads well on a screen. Read aloud verbatim it is
 * unbearable: a synthesiser will happily pronounce "hash hash Setup", spell a
 * path segment by segment, and recite three hundred lines of TypeScript. So the
 * text gets a pass over it before anything is synthesised.
 *
 * The second half is about latency rather than taste. Synthesis is per-call and
 * a long reply takes seconds to come back, so the text is cut into utterances
 * and the first one can be spoken while the rest are still being made. Sentence
 * boundaries are the natural cut because a sentence is what a voice knows how to
 * end.
 *
 * This module is deliberately pure and dependency-free, exactly like
 * `permission.js`: the backend synthesises, and everything in the window that
 * decides what to send it - the call view, auto-speak, the speak button -
 * reads this one answer to "what does this reply sound like".
 */

/**
 * The hard cap on one synthesis call.
 *
 * Past a couple of thousand characters a spoken reply has stopped being useful -
 * nobody listens to eight minutes of prose they could have skimmed - and every
 * provider charges by the character. Cutting here is cheaper than cutting after.
 */
export const MAX_SPEAK_CHARS = 2000;

/* -- markdown to speech -------------------------------------------------- */

/**
 * Fence languages worth naming out loud.
 *
 * A fence tag is written for a syntax highlighter, not for a listener, so `ts`
 * and `sh` get spoken names. Anything not in the table is said as written, which
 * is the right failure: an unusual language named oddly still beats silence.
 */
const LANGUAGE_NAMES = {
  bash: "a shell",
  c: "a C",
  cjs: "a JavaScript",
  cpp: "a C plus plus",
  cs: "a C sharp",
  css: "a CSS",
  diff: "a diff",
  go: "a Go",
  html: "an HTML",
  java: "a Java",
  js: "a JavaScript",
  json: "a JSON",
  jsx: "a JSX",
  kt: "a Kotlin",
  md: "a markdown",
  mjs: "a JavaScript",
  php: "a PHP",
  ps1: "a PowerShell",
  py: "a Python",
  python: "a Python",
  rb: "a Ruby",
  rs: "a Rust",
  sh: "a shell",
  shell: "a shell",
  sql: "a SQL",
  swift: "a Swift",
  toml: "a TOML",
  ts: "a TypeScript",
  tsx: "a TSX",
  typescript: "a TypeScript",
  xml: "an XML",
  yaml: "a YAML",
  yml: "a YAML",
  zsh: "a shell",
};

function codeBlockPlaceholder(language) {
  const tag = String(language ?? "")
    .trim()
    .toLowerCase();
  if (!tag) return "(a code block)";
  const name = LANGUAGE_NAMES[tag] ?? `a ${tag}`;
  return `(${name} code block)`;
}

/** Pictographs carry no sound, and a synthesiser asked to read one either says
 *  its CLDR name out loud or stalls. Variation selectors, joiners and skin tone
 *  modifiers go with them so a composed emoji leaves nothing behind. */
// The linter objects to the joiner and the variation selectors sharing a class
// with the pictographs, because splitting a composed emoji is usually a bug.
// Here it is the entire intention: a flag or a skin-toned figure has to lose
// every one of its parts, and leaving the joiner behind would feed the
// synthesiser a stray control character to stumble over.
const PICTOGRAPHIC =
  // eslint-disable-next-line no-misleading-character-class
  /[\p{Extended_Pictographic}\p{Regional_Indicator}\u{1F3FB}-\u{1F3FF}\uFE0E\uFE0F\u200D]/gu;

/** A separator row carries alignment, not data, so it is dropped rather than
 *  read as a run of dashes. */
const TABLE_SEPARATOR = /^\|?[\s:|-]*-[\s:|-]*\|?$/;

/**
 * A path is shortened to its last segment.
 *
 * "apps slash web slash src slash lib slash dictation dot ts" is noise: the
 * listener already knows roughly where they are, and the filename is the part
 * that identifies the thing. Two or more slashes and no whitespace is the test,
 * so a lone `a/b` ratio and any prose with spaces in it are left alone.
 */
const DEEP_PATH = /(?:[^\s/]+\/){2,}[^\s/]*/g;

function shortenPath(match) {
  const segments = match.split("/").filter(Boolean);
  return segments[segments.length - 1] ?? match;
}

function stripEmphasis(text) {
  return (
    text
      .replace(/\*\*([^*]+?)\*\*/g, "$1")
      .replace(/__([^_]+?)__/g, "$1")
      .replace(/~~([^~]+?)~~/g, "$1")
      .replace(/\*([^*\n]+?)\*/g, "$1")
      // A single underscore only counts as emphasis at a word edge, so
      // `snake_case_names` survives intact rather than being welded together.
      .replace(/(^|[\s(["'])_([^_\n]+?)_(?=[\s).,!?:;\]"']|$)/g, "$1$2")
  );
}

function speakableInline(text) {
  let out = text;

  // Images before links, because an image is a link with a bang in front of it
  // and the link rule would otherwise eat the alt text and leave the bang.
  out = out.replace(/!\[([^\]]*)\]\([^)]*\)/g, (_match, alt) => alt.trim() || "(an image)");
  out = out.replace(/\[([^\]]*)\]\([^)]*\)/g, (_match, label) => label.trim() || "(a link)");

  // Backticks are punctuation the listener cannot hear; the content inside them
  // is usually a symbol or a filename and is worth keeping.
  out = out.replace(/`+([^`]*)`+/g, "$1");

  // A bare URL is unspeakable - protocol, dots, slashes, query string - and the
  // fact that a link was there is the whole of what a listener needs.
  out = out.replace(/\b(?:https?:\/\/|www\.)\S+/gi, "(a link)");

  out = stripEmphasis(out);
  out = out.replace(DEEP_PATH, shortenPath);
  out = out.replace(PICTOGRAPHIC, "");
  return out;
}

function endsSentence(text) {
  return /[.!?…]["')\]]?$/.test(text.trim());
}

/**
 * Markdown in, plain speech out.
 *
 * Block structure is handled line by line and inline markup by rewrite, in that
 * order, because a `*` at the start of a line is a bullet and a `*` in the
 * middle of one is emphasis, and only position tells them apart.
 */
export function speakable(markdown) {
  const source = String(markdown ?? "").replace(/\r\n?/g, "\n");
  const lines = source.split("\n");
  const spoken = [];

  let fenceMarker = null;

  for (const line of lines) {
    const trimmed = line.trim();

    if (fenceMarker) {
      // Everything up to the closing fence is code, and code is never read.
      if (trimmed.startsWith(fenceMarker)) fenceMarker = null;
      continue;
    }

    const fence = trimmed.match(/^(```+|~~~+)(.*)$/);
    if (fence) {
      fenceMarker = fence[1].slice(0, 3);
      spoken.push(codeBlockPlaceholder(fence[2].trim().split(/\s+/)[0]));
      continue;
    }

    if (!trimmed) continue;

    // Blockquote markers are typography. Strip them and speak what was quoted.
    let text = trimmed.replace(/^(?:\s*>\s?)+/, "").trim();
    if (!text) continue;

    if (TABLE_SEPARATOR.test(text) && text.includes("|")) continue;

    if (text.startsWith("|")) {
      const cells = text
        .replace(/^\|/, "")
        .replace(/\|$/, "")
        .split("|")
        .map((cell) => speakableInline(cell).trim())
        .filter(Boolean);
      if (!cells.length) continue;
      // One row is one sentence, so the voice pauses between rows instead of
      // running a whole table together into a single breathless clause.
      const row = cells.join(", ");
      spoken.push(endsSentence(row) ? row : `${row}.`);
      continue;
    }

    const heading = text.match(/^#{1,6}\s+(.*)$/);
    if (heading) {
      const title = speakableInline(heading[1]).trim();
      if (!title) continue;
      // A heading has no punctuation of its own, so without one added it runs
      // straight into the paragraph beneath it as a single sentence.
      spoken.push(endsSentence(title) ? title : `${title}.`);
      continue;
    }

    text = text.replace(/^[-*+]\s+/, "").replace(/^\d+[.)]\s+/, "");
    // A thematic break is a horizontal rule, which sounds like nothing.
    if (/^([-*_])\1{2,}$/.test(text)) continue;

    const said = speakableInline(text).trim();
    if (said) spoken.push(said);
  }

  return spoken.join(" ").replace(/\s+/g, " ").trim();
}

/* -- utterances ---------------------------------------------------------- */

/**
 * Words that end in a period without ending a sentence.
 *
 * Splitting on "e.g." hands the synthesiser a one-word utterance and a sentence
 * that starts mid-thought, and the pause lands in the wrong place. This is not
 * every abbreviation in English, only the ones that actually show up in an
 * agent's prose.
 */
const ABBREVIATIONS = new Set([
  "e.g",
  "i.e",
  "etc",
  "vs",
  "approx",
  "dr",
  "mr",
  "mrs",
  "ms",
  "prof",
  "st",
  "no",
]);

function isAbbreviation(before) {
  const token = before.match(/[A-Za-z.]*[A-Za-z]$/)?.[0];
  if (!token) return false;
  // A single capital is an initial - "J. R. Tolkien" is one name, not three
  // sentences - and there is no way to tell it from a real sentence end, so the
  // reading that keeps the name whole wins.
  if (token.length === 1) return /[A-Z]/.test(token);
  return ABBREVIATIONS.has(token.toLowerCase());
}

const CLAUSE_BREAKS = [";", ":", ","];

/**
 * Cut an over-long sentence somewhere a listener would not notice.
 *
 * A clause boundary near the middle is best because the voice already pauses
 * there; a word boundary is the fallback; a hard cut mid-word is never done,
 * since half a word spoken aloud is not recoverable the way half a word on
 * screen is.
 */
function splitLong(text, maxChars) {
  if (text.length <= maxChars) return [text];

  const limit = Math.min(maxChars, text.length - 1);
  const target = Math.min(Math.ceil(text.length / 2), limit);

  let best = -1;
  for (let i = 1; i < limit; i++) {
    const isClause = CLAUSE_BREAKS.includes(text[i]) || text.slice(i, i + 3) === " - ";
    if (!isClause) continue;
    const cut = text[i] === " " ? i + 3 : i + 1;
    if (cut >= text.length) continue;
    if (best === -1 || Math.abs(cut - target) < Math.abs(best - target)) best = cut;
  }

  if (best === -1) best = text.lastIndexOf(" ", limit);
  if (best <= 0) best = limit;

  const head = text.slice(0, best).trim();
  const tail = text.slice(best).trim();
  if (!head || !tail) return [text];
  return [head, ...splitLong(tail, maxChars)];
}

/**
 * Plain text in, speakable chunks out.
 *
 * `minChars` exists because a two-word utterance spoken on its own sounds
 * clipped - the voice barely gets going before it stops - so a stray fragment is
 * glued onto whatever came before it instead.
 */
export function toUtterances(text, { minChars = 12, maxChars = 320 } = {}) {
  const source = String(text ?? "").trim();
  if (!source) return [];

  const sentences = [];
  let start = 0;

  for (let i = 0; i < source.length; i++) {
    const ch = source[i];
    if (ch !== "." && ch !== "!" && ch !== "?" && ch !== "…") continue;

    if (ch === ".") {
      const previous = source[i - 1] ?? "";
      const next = source[i + 1] ?? "";
      // "3.5 seconds" is one measurement and one sentence. A period between two
      // digits is a decimal point every time.
      if (/\d/.test(previous) && /\d/.test(next)) continue;
      if (isAbbreviation(source.slice(start, i))) continue;
    }

    // Trailing quotes and brackets belong to the sentence they close, so the
    // cut goes after them rather than orphaning them onto the next utterance.
    let end = i + 1;
    while (end < source.length && /[.!?…"')\]]/.test(source[end])) end++;

    const after = source[end] ?? "";
    if (after && !/\s/.test(after)) continue;

    const sentence = source.slice(start, end).trim();
    if (sentence) sentences.push(sentence);
    start = end;
  }

  const tail = source.slice(start).trim();
  if (tail) sentences.push(tail);

  const merged = [];
  for (const sentence of sentences) {
    if (merged.length && sentence.length < minChars) {
      merged[merged.length - 1] = `${merged[merged.length - 1]} ${sentence}`;
      continue;
    }
    merged.push(sentence);
  }

  return merged.flatMap((sentence) => splitLong(sentence, maxChars));
}

/* -- the whole job ------------------------------------------------------- */

/**
 * A markdown reply, ready to be spoken one utterance at a time.
 *
 * The truncation happens on the prose rather than on the utterance list so the
 * cut lands at a word boundary: a listener can tell that a reply was cut short,
 * but not that it was cut short in the middle of "authentication".
 */
export function speechFor(markdown, options) {
  let text = speakable(markdown);
  if (text.length > MAX_SPEAK_CHARS) {
    const cut = text.lastIndexOf(" ", MAX_SPEAK_CHARS);
    text = text.slice(0, cut > 0 ? cut : MAX_SPEAK_CHARS).trim();
  }
  return toUtterances(text, options);
}
