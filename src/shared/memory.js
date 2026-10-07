/**
 * What a memory is, and which ones matter right now.
 *
 * A memory is one durable fact worth carrying between conversations: how this
 * person wants to be worked with, what a project is for, a decision and the
 * reason behind it. One fact per record, deliberately - a record holding three
 * things cannot be superseded when one of them changes, and superseding is most
 * of what keeps a memory store from rotting into a pile of contradictions.
 *
 * The bargain is the same one skills make, for the same reason. A title and one
 * line for every memory go into the prompt, where they cost bytes on every turn;
 * the body arrives only when the model asks for it. So a title has to be worth
 * reading on its own, and `summarise` is what makes a long body behave.
 *
 * Pure and dependency-free. The window uses it to render and sort; scoring and
 * injecting happen in the Rust backend (the `inertia-memory` crate), whose
 * numbers these must match, or "which memories matter" gets two different
 * answers.
 */

/** A memory nobody has recalled in this long reads as stale. */
export const STALE_AFTER_DAYS = 21;

/**
 * How much of the prompt every turn may spend on memories it was not asked for.
 *
 * Two and a half kilobytes is roughly twenty entries at a title and a line each.
 * Picked against a measurement rather than a feeling: the tool-access work cut
 * about 34 KB of schema off a request, so this spends under a tenth of what was
 * saved, and a turn that needs more can ask for it.
 */
export const INJECT_BUDGET_BYTES = 2560;

/** How many entries are worth advertising however small they are. */
export const INJECT_MAX = 24;

/** One line of advertising copy, however long the body is. */
export const MAX_SUMMARY = 160;

/* -- scope ---------------------------------------------------------------- */

/**
 * Compare two folder paths as the same place.
 *
 * Windows makes this less obvious than it looks: the same folder legitimately
 * arrives as `D:\work\api` and `D:/work/api`, and the drive letter's case
 * varies with who produced the string. A memory that fails to match its own
 * project because of a slash would be invisible with no way for a user to see
 * why, which is worse than a memory that is merely missing.
 */
export function sameFolder(a, b) {
  const clean = (value) =>
    String(value ?? "")
      .replace(/[\\/]+/g, "/")
      .replace(/\/+$/, "")
      .toLowerCase();
  const left = clean(a);
  const right = clean(b);
  return Boolean(left) && left === right;
}

/**
 * The memories that apply while working in `folder`.
 *
 * Global memories always apply. A project memory applies only in its own
 * project. A project memory whose folder is missing - written before this
 * existed, or hand-edited - is treated as global, because the alternative is a
 * record that exists, is visible on screen, and can never be recalled by
 * anything.
 */
export function applicable(memories, folder) {
  return (memories ?? []).filter((memory) => {
    if (memory?.scope !== "project") return true;
    if (!memory?.folder) return true;
    return sameFolder(memory.folder, folder);
  });
}

/**
 * Memories that are actually in force.
 *
 * A memory waiting for approval exists, is visible on the Memory screen, and is
 * not yet believed by anything. Somebody who asked to review what gets written
 * down has not agreed to it being used in the meantime, and using it would make
 * the review a formality performed after the fact.
 */
export function approved(memories) {
  return (memories ?? []).filter((memory) => memory?.pending !== true);
}

/**
 * The one-liner shown in the prompt and nowhere else.
 *
 * Prefers an explicit `description`, because a person who wrote one meant it,
 * and otherwise takes the opening of the body. Newlines collapse: this sits
 * inside an XML element in a prompt and a stray one reads as structure.
 */
export function summarise(memory, limit = MAX_SUMMARY) {
  const written = String(memory?.description ?? "").replace(/\s+/g, " ").trim();
  const body = String(memory?.body ?? "").replace(/\s+/g, " ").trim();
  const line = written || body;
  if (!line) return "";
  return line.length > limit ? `${line.slice(0, limit).trimEnd()}...` : line;
}

/* -- scoring -------------------------------------------------------------- */

/**
 * Words, lowercased, with the ones that carry no signal dropped.
 *
 * The stop list is short on purpose. A long one is tuned to English prose, and
 * these are titles and tags written by a model and a person in a hurry, where
 * throwing away half the words to save a few bytes of index costs more matches
 * than it saves noise.
 */
const STOP = new Set([
  "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "how",
  "in", "is", "it", "of", "on", "or", "that", "the", "this", "to", "was", "what",
  "when", "where", "which", "with",
]);

export function terms(text) {
  return String(text ?? "")
    .toLowerCase()
    .split(/[^a-z0-9_]+/)
    .filter((word) => word.length > 1 && !STOP.has(word));
}

/**
 * The searchable text of one memory, with the parts that matter counted twice.
 *
 * A word in the title or a tag says more about what a memory is *for* than the
 * same word buried in the body, and repeating the field is how a bag-of-words
 * model is told so without a second set of weights to keep in step.
 */
export function textOf(memory) {
  return [
    memory?.title,
    memory?.title,
    (memory?.tags ?? []).join(" "),
    (memory?.tags ?? []).join(" "),
    memory?.description,
    memory?.body,
  ]
    .filter(Boolean)
    .join(" ");
}

/** BM25's two dials, at the values everyone uses because they work. */
const K1 = 1.2;
const B = 0.75;

/**
 * Build the index once for a set of memories.
 *
 * Held by the caller and thrown away when a memory changes. At the scale a
 * personal workspace reaches - hundreds, maybe thousands - this is microseconds
 * and there is nothing to gain from anything cleverer. A vector database earns
 * its keep at a million records; here it would be a service to install, back up
 * and explain, in exchange for nothing anyone could measure.
 */
export function index(memories) {
  const docs = (memories ?? []).map((memory) => {
    const words = terms(textOf(memory));
    const counts = new Map();
    for (const word of words) counts.set(word, (counts.get(word) ?? 0) + 1);
    return { memory, counts, length: words.length };
  });

  const seen = new Map();
  for (const doc of docs) {
    for (const word of doc.counts.keys()) seen.set(word, (seen.get(word) ?? 0) + 1);
  }

  const total = docs.length || 1;
  const averageLength = docs.reduce((sum, doc) => sum + doc.length, 0) / total || 1;

  return { docs, seen, total, averageLength };
}

/** The classic inverse document frequency, floored so a common word cannot go negative. */
function idf(seen, total, word) {
  const n = seen.get(word) ?? 0;
  return Math.max(0.05, Math.log(1 + (total - n + 0.5) / (n + 0.5)));
}

/**
 * How long ago, as a number between 0 and 1 that decays over a season.
 *
 * Gentle on purpose: a memory from March is not wrong, it is just less likely to
 * be what this conversation is about, and a sharp decay would bury facts that
 * are still true because nobody happened to need them recently.
 */
function freshness(iso, now) {
  const at = Date.parse(iso ?? "");
  if (!Number.isFinite(at)) return 0;
  const days = (now - at) / 86400000;
  if (days <= 0) return 1;
  return 1 / (1 + days / 90);
}

/**
 * Score every memory against a query, best first.
 *
 * Three parts, and the ordering between them is the opinion: relevance decides,
 * a pin lifts, recency breaks ties. A pinned memory is the user saying "this one
 * always matters", so it gets a real boost rather than a rounding error - but
 * not so much that a pin outranks a memory that actually answers the question,
 * because then pinning six things would be the same as pinning none.
 */
export function score(built, query, { now = Date.now(), pinWeight = 1.5, freshWeight = 0.6 } = {}) {
  const words = terms(query);
  const { docs, seen, total, averageLength } = built;

  return docs
    .map(({ memory, counts, length }) => {
      let relevance = 0;
      for (const word of words) {
        const count = counts.get(word) ?? 0;
        if (!count) continue;
        const norm = count * (K1 + 1);
        const denom = count + K1 * (1 - B + (B * length) / averageLength);
        relevance += idf(seen, total, word) * (norm / denom);
      }

      const fresh = freshness(memory?.lastUsedAt || memory?.updatedAt || memory?.createdAt, now);
      const pinned = memory?.pinned ? pinWeight : 0;

      return { memory, relevance, score: relevance + pinned + fresh * freshWeight };
    })
    .sort((a, b) => b.score - a.score || String(a.memory?.id).localeCompare(String(b.memory?.id)));
}

/**
 * The memories a query is actually about.
 *
 * `relevance > 0` is the whole filter and it matters: without it every query
 * returns the pinned memories plus whatever is newest, which looks like recall
 * working and is in fact recall having nothing to say. A pinned memory reaches
 * the model through the injected index regardless.
 */
export function search(memories, query, { limit = 8, now = Date.now() } = {}) {
  if (!String(query ?? "").trim()) return [];
  return score(index(memories), query, { now })
    .filter((row) => row.relevance > 0)
    .slice(0, limit)
    .map((row) => row.memory);
}

/**
 * What goes in the prompt when nobody has asked anything yet.
 *
 * Pinned first, then whatever has proved useful, then whatever is new - and cut
 * to a byte budget rather than a count, because twenty terse memories and twenty
 * verbose ones are not the same purchase.
 */
export function forPrompt(
  memories,
  { query = "", budget = INJECT_BUDGET_BYTES, max = INJECT_MAX, now = Date.now() } = {}
) {
  const list = memories ?? [];

  /**
   * Three tiers, and the middle one is the point.
   *
   * Pinned memories and the handover note go first whatever is being asked:
   * the user chose them, or they are what the last conversation left behind.
   * Then the memories that are actually *about* what was just said, scored the
   * same way recall scores them. Then, with whatever budget is left, the ones
   * that have proved useful before - a small baseline for the case where the
   * message shares no words with a memory that matters.
   *
   * Without the middle tier this was a popularity contest: a hundred memories
   * meant the twenty most-used went in regardless of the question, and the
   * one that answered it stayed on disk.
   */
  const relevance = new Map();
  if (String(query).trim()) {
    for (const row of score(index(list), query, { now, pinWeight: 0, freshWeight: 0 })) {
      if (row.relevance > 0) relevance.set(row.memory, row.relevance);
    }
  }

  const tier = (memory) => {
    if (memory?.pinned || memory?.kind === "handover") return 0;
    if (relevance.has(memory)) return 1;
    return 2;
  };

  const ranked = [...list].sort((a, b) => {
    const byTier = tier(a) - tier(b);
    if (byTier) return byTier;
    const byRelevance = (relevance.get(b) ?? 0) - (relevance.get(a) ?? 0);
    if (byRelevance) return byRelevance;
    const used = (b.useCount ?? 0) - (a.useCount ?? 0);
    if (used) return used;
    return freshness(b.lastUsedAt || b.updatedAt || b.createdAt, now) -
      freshness(a.lastUsedAt || a.updatedAt || a.createdAt, now);
  });

  const kept = [];
  let spent = 0;
  for (const memory of ranked) {
    if (kept.length >= max) break;
    const line = `${memory?.title ?? ""}: ${summarise(memory)}`;
    // A pinned memory is never dropped for being long. The user pinned it; the
    // budget is for deciding what else fits around it.
    if (spent + line.length > budget && !memory?.pinned && kept.length) break;
    kept.push(memory);
    spent += line.length;
  }
  return kept;
}

/* -- duplicates ----------------------------------------------------------- */

/**
 * How much two pieces of text are the same, from 0 to 1.
 *
 * Overlap of the words that carry meaning, ignoring order and repetition. Crude,
 * and right for this: what it has to catch is the same fact written twice, which
 * is a case where the words really are the same.
 */
export function similarity(a, b) {
  const left = new Set(terms(a));
  const right = new Set(terms(b));
  if (!left.size || !right.size) return 0;

  let shared = 0;
  for (const word of left) if (right.has(word)) shared += 1;
  return shared / (left.size + right.size - shared);
}

/** Titles compared the way a person would: case and punctuation do not count. */
function normalisedTitle(memory) {
  return String(memory?.title ?? "").toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
}

/**
 * How alike two memories must be before they are the same memory.
 *
 * High on purpose, and the asymmetry is deliberate: a near-duplicate that slips
 * through is untidy, while a wrong merge silently destroys a fact. "Allergic to
 * eggs" and "allergic to peanuts" are both true and score about a third, nowhere
 * near this.
 */
const SAME_MEMORY = 0.8;

/**
 * The memory a new one would be a repeat of, if there is one.
 *
 * Scoped first: two projects are allowed to hold the same sentence, because in
 * each one it is a fact about that project. Then an identical title is taken as
 * the same fact even when the body has changed - a body that changed under the
 * same title is a correction, and keeping both is how a store starts
 * contradicting itself.
 */
export function findDuplicate(memories, candidate, threshold = SAME_MEMORY) {
  const title = normalisedTitle(candidate);
  const text = `${candidate?.title ?? ""} ${candidate?.body ?? ""}`;

  for (const memory of memories ?? []) {
    const sameScope = (memory?.scope ?? "global") === (candidate?.scope ?? "global");
    if (!sameScope) continue;
    if (candidate?.scope === "project" && !sameFolder(memory?.folder, candidate?.folder)) continue;

    if (title && normalisedTitle(memory) === title) return memory;
    if (similarity(text, `${memory?.title ?? ""} ${memory?.body ?? ""}`) >= threshold) return memory;
  }
  return null;
}

/**
 * An older memory brought up to date by a newer one.
 *
 * The wording is replaced, because the newer text is the correction. Everything
 * a memory has *accumulated* is kept: its tags, its pin, when it was first
 * written, and how often it has been useful. Overwriting those was a quiet bug -
 * saving the same fact again with no tags wiped the tags somebody had added, so
 * a memory got worse every time it was confirmed.
 *
 * Tags are unioned rather than replaced for the same reason: a new pass that
 * happens to think of one tag should add it, not throw the other four away.
 */
export function merged(previous, next) {
  const tags = [];
  for (const tag of [...(previous?.tags ?? []), ...(next?.tags ?? [])]) {
    const clean = String(tag ?? "").trim();
    if (clean && !tags.some((one) => one.toLowerCase() === clean.toLowerCase())) tags.push(clean);
  }

  return {
    ...previous,
    ...next,
    tags: tags.slice(0, 8),
    // Both of these are the user's, not the writer's: a pin stays pinned, and a
    // memory keeps the day it was first written down.
    pinned: Boolean(previous?.pinned || next?.pinned),
    createdAt: previous?.createdAt ?? next?.createdAt,
    useCount: previous?.useCount ?? 0,
  };
}

/* -- hygiene -------------------------------------------------------------- */

export function isStale(memory, { now = Date.now(), days = STALE_AFTER_DAYS } = {}) {
  const at = Date.parse(memory?.lastUsedAt ?? "");
  if (!Number.isFinite(at)) return false;
  return now - at > days * 86400000;
}

/**
 * Things that must never be written into a memory.
 *
 * A capture pass reads whatever went past in a conversation, and what goes past
 * in a coding conversation includes keys. A memory is injected into every later
 * prompt and rendered on a screen, so a key that lands in one is a key that
 * leaks quietly and repeatedly. The patterns catch the shapes that are
 * unmistakable rather than trying to be clever: a false positive costs one
 * memory nobody needed, a false negative costs a credential.
 */
const SECRET_SHAPES = [
  /\bsk-[A-Za-z0-9_-]{16,}/,
  /\bgh[pousr]_[A-Za-z0-9]{16,}/,
  /\bxox[abps]-[A-Za-z0-9-]{10,}/,
  /\bAKIA[0-9A-Z]{16}\b/,
  /\bAIza[0-9A-Za-z_-]{30,}/,
  /-----BEGIN [A-Z ]*PRIVATE KEY-----/,
  /\bey[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}/,
  // A named field with a long opaque value after it, which is what a
  // hand-written config line looks like.
  /\b(?:api[_-]?key|secret|token|password|passwd)\b\s*[:=]\s*["']?[A-Za-z0-9_\-/+.]{16,}/i,
];

export function looksSecret(text) {
  const value = String(text ?? "");
  return SECRET_SHAPES.some((shape) => shape.test(value));
}

/** True when this memory is safe to store. Checked before writing, never after. */
export function isStorable(memory) {
  if (!String(memory?.title ?? "").trim()) return false;
  if (!String(memory?.body ?? "").trim()) return false;
  return !looksSecret(`${memory.title}\n${memory.body}`);
}

/* -- what to write down --------------------------------------------------- */

/**
 * The instructions a capture pass follows.
 *
 * Editable in settings, because what is worth remembering is a matter of taste
 * and of what the person is doing: a team lead wants decisions and conventions,
 * someone debugging wants symptoms and dead ends. This is the default, and it
 * is written to be good enough that nobody has to touch it.
 *
 * The shape of the advice matters more than its length. "Save important things"
 * produces a store full of nothing; naming what to skip is what keeps it small,
 * so the exclusions here are as specific as the inclusions.
 */
export const DEFAULT_INSTRUCTIONS = [
  "Write down what someone returning to this project in a month would need to",
  "know, and nothing else.",
  "",
  "Worth remembering:",
  "- Decisions, and the reasoning behind them. A decision without its reason",
  "  gets reversed by the next person who sees it.",
  "- Conventions this project follows that are not obvious from one file.",
  "- What a component or subsystem is for, when the name does not say it.",
  "- Bugs that were found and how they were actually caused.",
  "- Work that is unfinished, and what is left to do.",
  "- How this person wants to be worked with, when they say so.",
  "- Corrections they made to you. Those are the most valuable of all.",
  "",
  "Not worth remembering:",
  "- Anything readable from the code, the file listing or the git history.",
  "- What happened step by step. The outcome is the memory, not the path.",
  "- Anything already written in AGENTS.md. It is read on every turn.",
  "- Transient state: what is currently failing, what a command just printed,",
  "  what is on screen.",
  "- Pleasantries, restatements and anything you are unsure was ever agreed.",
  "",
  "One fact per memory. A memory holding three things cannot be corrected when",
  "one of them changes. Write titles somebody could pick out of a list of a",
  "hundred, and bodies that still make sense with none of this conversation",
  "around them. Prefer saving nothing to saving something vague.",
].join("\n");

/**
 * How often the app looks for something worth keeping.
 *
 * `session` is the default and the honest recommendation: it costs one model
 * call when a conversation goes quiet, and it sees a whole arc of work rather
 * than one step of it, which is exactly the difference between "renamed a file"
 * and "moved the routes because the old layout could not express nesting".
 */
export const MEMORY_CAPTURE = [
  {
    id: "session",
    label: "When a conversation ends",
    hint: "One pass over the whole conversation. The best of both, and what most people want.",
  },
  {
    id: "turn",
    label: "After every message",
    hint: "Catches things soonest. Costs a call per message and writes more that is not worth keeping.",
  },
  {
    id: "off",
    label: "Only when asked",
    hint: "Nothing is written down unless you or the agent decides to. Nothing happens in the background.",
  },
];
