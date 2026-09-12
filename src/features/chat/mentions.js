/*
 * mentions - what `@agent` and `/skill` mean in the box the user types in.
 *
 * The composer draws these as pills, but the VALUE it holds is still a plain
 * string with the tokens in it. That is deliberate and it is the whole design:
 * the send path, the saved draft, the resumed conversation and the main
 * process all keep working against the same string they always had, and the
 * pills are a rendering of it rather than a second data model to keep in step.
 *
 * Everything here is pure string work, so it can be tested. The DOM half - the
 * pills, the caret, the palette - lives in mention-dom.js, which cannot be,
 * because the tests run in node with no document.
 */

/**
 * What can be written into the box, and what starts each.
 *
 * Two kinds share the slash. A skill is a document in the workspace and a
 * command is a thing this app does, and to the person typing them they are
 * one list under one key - so the palette is keyed on the SIGIL rather than
 * on the kind, and `kind` survives only for the code downstream that has to
 * tell a skill from a command once one has been picked.
 */
export const SIGILS = { agent: "@", skill: "/", command: "/" };

/**
 * A name as it appears after a sigil.
 *
 * Lowercased and hyphenated so "Nova Reyes" is one token rather than a mention
 * of "Nova" followed by the word "Reyes". Case is dropped from the token but
 * not from the label: the pill still reads "Nova Reyes".
 */
export function slugOf(name) {
  return String(name ?? "")
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "");
}

/**
 * Everything that may draw as a pill, from the lists the app already has.
 *
 * `id` is what the mention resolves to; `raw` is what is in the text. Two
 * agents that slug the same - "Nova" and "nova" - would produce two specs with
 * one token, and the first wins, which is the same answer the user gets by
 * looking at the palette and picking the top one.
 */
export function mentionSpecs({ agents = [], skills = [], commands = [], face = null } = {}) {
  const specs = [];
  const seen = new Set();

  const add = (kind, id, name, look) => {
    const slug = slugOf(name);
    if (!slug) return;
    const raw = `${SIGILS[kind]}${slug}`;
    if (seen.has(raw)) return;
    seen.add(raw);
    specs.push({ raw, kind, id, label: String(name), ...look });
  };

  // A pill wants a face. A picture if the agent has one; otherwise its
  // initials on its own colour, which is what the avatar falls back to
  // everywhere else. `icon` is deliberately not used here - it names a glyph
  // in the icon set, not a file, and putting it in an `src` draws a broken
  // image where the agent should be.
  //
  // `face`, when given, knows more than this file does - it can read the
  // picture cache and draw the icon set - and whatever it returns wins.
  for (const agent of agents) {
    add("agent", agent.id, agent.name, {
      iconSrc: agent.avatarUrl ?? null,
      iconHtml: null,
      initials: agent.initials ?? String(agent.name ?? "").trim().charAt(0).toUpperCase(),
      tint: agent.avatarColor ?? null,
      ...(face ? face(agent) : {}),
    });
  }
  // Commands before skills, and it matters: both live under `/`, both are
  // matched by the same query, and a workspace skill called "compact" must
  // not quietly take the token that summarises the conversation.
  for (const command of commands) {
    add("command", command.name, command.name, {
      iconSrc: null,
      iconHtml: null,
      initials: null,
      tint: null,
      hint: command.summary ?? null,
      argHint: command.argHint || null,
    });
  }
  for (const skill of skills) {
    add("skill", skill.id ?? skill.name, skill.name, { iconSrc: null, iconHtml: null, initials: null, tint: null });
  }
  return specs;
}

/**
 * Every mention in `text`, longest token first so `/brand-guidelines-pro` is
 * not shadowed by `/brand-guidelines`.
 *
 * A token only counts at a word boundary: preceded by whitespace or the start
 * of the message, and not run into a letter or digit. Without the first rule an
 * email address would attach an agent; without the second, `@nova` would match
 * inside `@novabuild`.
 */
export function findSpecRanges(text, specs) {
  const found = [];
  const haystack = String(text ?? "");
  const lower = haystack.toLowerCase();
  const sorted = [...specs].filter((spec) => spec.raw.length > 1).sort((a, b) => b.raw.length - a.raw.length);

  for (const spec of sorted) {
    const needle = spec.raw.toLowerCase();
    let from = 0;
    for (;;) {
      const start = lower.indexOf(needle, from);
      if (start === -1) break;
      const end = start + needle.length;
      const openOk = start === 0 || /\s/.test(haystack[start - 1]);
      const closeOk = end >= haystack.length || !/[\p{L}\p{N}]/u.test(haystack[end]);
      const overlaps = found.some((range) => start < range.end && end > range.start);
      if (openOk && closeOk && !overlaps) found.push({ start, end, spec });
      from = end;
    }
  }
  return found.sort((a, b) => a.start - b.start);
}

/**
 * True when the caret sits inside a mention that has already become a pill.
 *
 * The palette's own detector works on raw text, where a finished `/review`
 * looks exactly like someone halfway through typing one. Accept a skill, press
 * backspace to remove the trailing space, and the palette would reopen
 * offering the thing just picked. A pill is a decision already made.
 */
export function caretInMention(text, specs, caret) {
  return findSpecRanges(text, specs).some((range) => caret > range.start && caret <= range.end);
}

/** How far a query may run before it stops being one. */
const MAX_QUERY = 40;

/**
 * The token being typed at `caret`, if there is one.
 *
 * Returns the sigil, what has been typed after it, and the span to replace
 * when something is picked. Returns null when the caret is not in a token -
 * which includes sitting inside a finished pill, and includes a `/` in the
 * middle of a path, because a sigil only opens a palette at a word boundary.
 */
export function activeQuery(text, caret, specs = []) {
  const haystack = String(text ?? "");
  const at = Math.max(0, Math.min(caret ?? 0, haystack.length));
  if (caretInMention(haystack, specs, at)) return null;

  for (let start = at - 1; start >= 0 && at - start <= MAX_QUERY + 1; start -= 1) {
    const char = haystack[start];
    // Whitespace before the sigil means there is no token open here at all.
    if (/\s/.test(char)) return null;
    const kind = Object.keys(SIGILS).find((name) => SIGILS[name] === char);
    if (!kind) continue;
    // `kind` is the first one that claims this character and is kept only
    // because callers have always had it; `sigil` is what the matching uses.
    if (start > 0 && !/\s/.test(haystack[start - 1])) return null;
    const query = haystack.slice(start + 1, at);
    // A space inside the query closes it: "@nova can you" is a mention
    // followed by prose, not a search for "nova can you".
    if (/\s/.test(query)) return null;
    return { kind, sigil: char, query, start, end: at };
  }
  return null;
}

/**
 * Rank the specs of one kind against what has been typed.
 *
 * A prefix match comes before a match in the middle, and within each group the
 * shorter label wins, so typing "no" offers "Nova" before "Nova Reyes" - the
 * shorter name is the one more likely to be finished by a keystroke or two.
 */
export function matchMentions(specs, { kind, sigil, query }) {
  const needle = String(query ?? "").toLowerCase();
  const wanted = sigil ?? SIGILS[kind];
  const pool = specs.filter((spec) => SIGILS[spec.kind] === wanted);
  if (!needle) return pool;

  return pool
    .map((spec) => {
      const token = spec.raw.slice(1);
      const label = spec.label.toLowerCase();
      if (token.startsWith(needle) || label.startsWith(needle)) return { spec, rank: 0 };
      if (token.includes(needle) || label.includes(needle)) return { spec, rank: 1 };
      return null;
    })
    .filter(Boolean)
    .sort(
      (a, b) =>
        a.rank - b.rank ||
        // A tie between a command and a skill goes to the command: it is the
        // one this app promises works, and the one the name was reserved for.
        (a.spec.kind === "command" ? 0 : 1) - (b.spec.kind === "command" ? 0 : 1) ||
        a.spec.label.length - b.spec.label.length
    )
    .map((hit) => hit.spec);
}

/**
 * Put a picked mention into the text, and say where the caret goes.
 *
 * A trailing space is added because the next thing typed is prose, and without
 * it the token would grow a word on the end and stop being a token.
 */
export function applyMention(text, range, spec) {
  const haystack = String(text ?? "");
  const before = haystack.slice(0, range.start);
  const after = haystack.slice(range.end);
  const spaced = after.startsWith(" ") ? after : ` ${after}`;
  const next = `${before}${spec.raw}${spaced}`;
  return { text: next, caret: before.length + spec.raw.length + 1 };
}

/** The ids mentioned in `text`, by kind, in the order they were written. */
export function mentionedIds(text, specs) {
  const out = { agent: [], skill: [], command: [] };
  for (const { spec } of findSpecRanges(text, specs)) {
    const list = out[spec.kind];
    if (list && !list.includes(spec.id)) list.push(spec.id);
  }
  return out;
}
