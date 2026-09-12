/**
 * How much of a long conversation is in the document at once.
 *
 * A transcript grew without limit: every message of every exchange stayed in
 * the DOM for as long as the thread was open, and a reply that used fifty tools
 * is fifty cards on top of its prose. A hundred-message conversation is tens of
 * thousands of nodes that nobody is looking at, all of them still costing style
 * recalculation and layout on every token of the reply being written at the
 * bottom.
 *
 * ── Why a window and not virtualisation ────────────────────────────────────
 * Proper virtualisation measures every row and positions it absolutely. That
 * needs a stable height per row, and this list has none: a message grows token
 * by token while it streams, a tool card changes height when it is opened, a
 * picture changes it again when it loads. Every one of those is a measurement
 * invalidated after the fact, and the failure mode is the transcript jumping
 * under the reader - which is worse than the problem being solved.
 *
 * A window has none of that. The recent messages - the ones anybody is actually
 * reading - render exactly as they always did, and the older ones are behind a
 * button. Nothing is measured, nothing is positioned, and the part of the list
 * that changes constantly is the part that was never windowed.
 */

/** Messages kept in the document, and how many more a click adds. */
export const WINDOW_SIZE = 30;

/**
 * The tail of a transcript, and what was left out.
 *
 * The tail rather than the head: a conversation is read from the bottom, and
 * the newest exchange is the one being written into.
 *
 * `shown` is a count rather than an index so it survives the list growing - a
 * turn that appends five tool results while the reader is looking at older
 * messages must not shift what they can see.
 */
export function windowOf(blocks = [], shown = WINDOW_SIZE) {
  const list = Array.isArray(blocks) ? blocks : [];
  // A count that is not a positive number falls back to the default rather
  // than to one: the caller has a bug, and showing a single message is a much
  // stranger thing to do about it than showing the usual window.
  const asked = Math.floor(Number(shown));
  const keep = Number.isFinite(asked) && asked > 0 ? asked : WINDOW_SIZE;
  if (list.length <= keep) return { visible: list, hidden: 0 };
  return { visible: list.slice(list.length - keep), hidden: list.length - keep };
}

/**
 * Whether a thread is long enough for the window to be worth mentioning.
 *
 * Below this the button would be a control that appears once, does nothing
 * anyone asked for, and never comes back.
 */
export function isWindowed(blocks = [], shown = WINDOW_SIZE) {
  return windowOf(blocks, shown).hidden > 0;
}
