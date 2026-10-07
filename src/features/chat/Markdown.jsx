import * as React from "react";
import { cn } from "@/lib/utils";
import { parseBlocks } from "./markdown/parse-blocks.js";
import { MemoBlock, BlockList } from "./markdown/Block.jsx";
import { FootnoteNumbers } from "./markdown/footnotes.js";

/**
 * The markdown renderer for message prose.
 *
 * It parses to React elements - never `dangerouslySetInnerHTML`, because
 * message content is model output and must never be able to inject markup.
 * The grammar lives in `markdown/parse-blocks.js` and `markdown/parse-inline.js`
 * as pure functions so it can be tested without a DOM; this file is only the
 * mapping from blocks to elements, plus the streaming bookkeeping below.
 *
 * ── Why this is not just `blocks.map(render)` ──────────────────────────────
 * A streaming reply re-parses on every token, which is cheap. Re-rendering
 * every block on every token is not: a long answer with several code blocks
 * and a table would rebuild all of them sixty times a second.
 *
 * Two things keep that to one block per token:
 *
 *  · Identity. `useStableBlocks` hands back the *previous* object for any
 *    block whose content is unchanged, so `React.memo`'s pointer comparison
 *    short-circuits every block above the one being typed.
 *  · Keys. The key is position plus block type, and deliberately not content.
 *    Position alone is wrong because a block can change type under its own
 *    index while streaming - a header line of pipes becomes a table the moment
 *    the alignment row lands - and reusing the DOM across that change is
 *    exactly the flicker we are avoiding. Content is wrong because the last
 *    block's content changes on every token, and keying on it would remount
 *    that block each time, losing the code block's copy state and its scroll
 *    position.
 */

/**
 * Whether two parsed blocks describe the same thing.
 *
 * This used to be `JSON.stringify(a) === JSON.stringify(b)`, which is correct
 * and was quietly enormous: it ran over every block on every token, so a reply
 * carrying a 400-line file built and threw away about a hundred megabytes of
 * strings on its way to the screen - measured, not guessed - and the figure
 * rises with the square of the reply. All of it to answer a question that
 * needs no strings at all.
 *
 * A hand-written walk allocates nothing and stops at the first difference,
 * which for a streaming reply is usually the first field of the last block.
 * It stays exact: the alternative of comparing a cheap fingerprint would
 * eventually hold two different blocks to be equal and render a stale one.
 */
function same(a, b) {
  if (a === b) return true;
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") return false;

  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i += 1) if (!same(a[i], b[i])) return false;
    return true;
  }
  if (Array.isArray(b)) return false;

  const keys = Object.keys(a);
  if (keys.length !== Object.keys(b).length) return false;
  for (const key of keys) {
    if (!Object.prototype.hasOwnProperty.call(b, key)) return false;
    if (!same(a[key], b[key])) return false;
  }
  return true;
}

function useStableBlocks(blocks) {
  const previous = React.useRef([]);
  // an unchanged block keeps its old object, so memo sees the same pointer
  const next = blocks.map((block, index) => {
    const old = previous.current[index];
    return old !== undefined && same(old, block) ? old : block;
  });
  previous.current = next;
  return next;
}

/**
 * The notes at the foot of a message.
 *
 * Footnote definitions are pulled out of the block stream rather than rendered
 * where they were written, because a model writes them wherever it likes and a
 * line reading `[^1]: ...` in the middle of an answer is not what the author
 * meant by a footnote. They are numbered in the order they were written, which
 * is the order a reader meets them.
 */
function Footnotes({ notes }) {
  if (!notes.length) return null;
  return (
    <div className="mt-4 border-t border-border-subtle pt-2 text-[0.92em] text-muted-foreground">
      {notes.map((note, index) => (
        <div key={note.id} className="flex gap-2 py-0.5">
          <span className="shrink-0 tabular-nums">[{index + 1}]</span>
          <div className="min-w-0 flex-1">
            <BlockList blocks={note.blocks} tight />
          </div>
        </div>
      ))}
    </div>
  );
}

export function Markdown({ children, className, streaming = false }) {
  const parsed = React.useMemo(() => parseBlocks(children, { streaming }), [children, streaming]);
  const notes = React.useMemo(() => parsed.filter((block) => block.type === "footnote"), [parsed]);
  const numbers = React.useMemo(
    () => new Map(notes.map((note, index) => [note.id, index + 1])),
    [notes]
  );
  const blocks = useStableBlocks(parsed.filter((block) => block.type !== "footnote"));
  const lastIndex = blocks.length - 1;

  // chat-text, not text-base: the reader's chosen size has to reach the prose
  // itself, and a fixed size utility here would silently win over anything set
  // on an ancestor.
  return (
    <FootnoteNumbers.Provider value={numbers}>
      <div className={cn("chat-text min-w-0 leading-relaxed text-foreground", className)}>
        {blocks.map((block, index) => (
          <MemoBlock
            key={`${index}-${block.type}`}
            block={block}
            caret={streaming && index === lastIndex}
          />
        ))}
        <Footnotes notes={notes} />
      </div>
    </FootnoteNumbers.Provider>
  );
}
