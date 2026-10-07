import React from "react";
import { Popover } from "@/components/ui/popover";
import { Icon } from "@/components/icons";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { cn } from "@/lib/utils";
import { activeQuery, applyMention, matchMentions } from "./mentions.js";
import { caretRect, offsetRect } from "./mention-dom.js";

/*
 * The list that opens under `@` and `/`.
 *
 * Anchored to the CARET, not to the composer. A list pinned to the corner of
 * the field is fine while the field is one line tall and wrong the moment it
 * is not: type six lines, press `@` on the last, and the choices appear at the
 * top of the box a long way from the cursor, reading as an unrelated panel.
 *
 * Focus stays in the editor the whole time. A palette that took focus would
 * end the typing that opened it - the next character would go nowhere - so the
 * arrow keys are handled by the composer's own key handler and passed down
 * here. That is why this is a hook plus a presentational component rather than
 * a self-contained menu.
 */

/** How many rows are offered at once. Enough to choose from, few enough to read. */
const MAX_ROWS = 8;

/**
 * Track the token being typed and what it could become.
 *
 * The value and caret come from the editor rather than being read off the DOM,
 * because the two disagree for one frame after every edit and the palette must
 * follow the text, not the paint.
 */
export function useMentionPalette({ mentions, editorRef, nodeRef, onChange }) {
  const [state, setState] = React.useState(null);
  const [index, setIndex] = React.useState(0);
  // The anchor below is a closure made once; this is how it sees the token
  // that is open now rather than the one that was open when it was made.
  const stateRef = React.useRef(null);
  stateRef.current = state;

  /**
   * A virtual anchor: an object with a rect rather than a node, which is what
   * `useAnchoredPosition` asks for. The caret has no element of its own.
   *
   * Measured on demand rather than frozen when the palette opens. A frozen rect
   * is only right if it was taken at exactly the right moment, and there are
   * several ways for it not to be - the `+` menu types the `@` for you while
   * focus is still moving, a repaint lands between the keystroke and the
   * measurement, the editor reflows as the composer grows a line. Any of those
   * pins the list somewhere the cursor is not. Measuring when asked is right
   * every time, and it costs one `getClientRects` per placement.
   *
   * The last good rect is kept as a fallback for the moments the caret cannot
   * be measured at all - during a repaint, or when focus has just left.
   */
  const lastRect = React.useRef(new DOMRect(0, 0, 0, 0));
  const anchorRef = React.useRef({
    getBoundingClientRect: () => {
      const node = nodeRef.current;
      const range = stateRef.current?.range;
      // The sigil itself, so the list stays put while the query is typed
      // after it. Falls back to the live caret, then to the last good answer.
      const rect =
        (node && range ? offsetRect(node, range.start) : null) ?? (node ? caretRect(node) : null);
      if (rect && rect.height > 0) lastRect.current = rect;
      return lastRect.current;
    },
  });

  const close = React.useCallback(() => setState(null), []);

  const track = React.useCallback(
    (value, caret) => {
      const range = activeQuery(value, caret, mentions);
      if (!range) {
        setState(null);
        return;
      }
      const items = matchMentions(mentions, range).slice(0, MAX_ROWS);
      if (items.length === 0) {
        // Nothing matches what has been typed. Closing rather than showing an
        // empty box: `@` is a perfectly ordinary character and someone writing
        // an email address should not have a panel following them across it.
        setState(null);
        return;
      }
      setState({ range, items, value });
      setIndex((current) => (current < items.length ? current : 0));
    },
    [mentions]
  );

  const accept = React.useCallback(
    (spec) => {
      if (!state || !spec) return false;
      const next = applyMention(state.value, state.range, spec);
      setState(null);
      onChange(next.text);
      // Asked for before the value has arrived: the editor holds it until its
      // own repaint, because this offset means nothing in the text on screen
      // right now. See PromptEditor's pending caret.
      editorRef.current?.setCaret?.(next.caret);
      return true;
    },
    [state, onChange, editorRef]
  );

  /**
   * The keys the palette owns while it is open.
   *
   * Returns true when it handled the event, which is the composer's signal not
   * to treat Enter as send or Escape as stop.
   */
  const onKeyDown = React.useCallback(
    (event) => {
      if (!state) return false;
      const { items } = state;

      if (event.key === "ArrowDown") {
        setIndex((i) => (i + 1) % items.length);
        return true;
      }
      if (event.key === "ArrowUp") {
        setIndex((i) => (i - 1 + items.length) % items.length);
        return true;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        return accept(items[index] ?? items[0]);
      }
      if (event.key === "Escape") {
        close();
        return true;
      }
      return false;
    },
    [state, index, accept, close]
  );

  return {
    open: Boolean(state),
    items: state?.items ?? [],
    kind: state?.range?.kind ?? null,
    index,
    setIndex,
    anchorRef,
    track,
    accept,
    close,
    onKeyDown,
  };
}

/**
 * One row: a face, a name, and the token it will leave in the text.
 *
 * The face is the agent's real one, drawn by the same component the header and
 * the transcript use, so the picture somebody set is the picture they see when
 * they type an `@`. A skill has no face and gets the glyph the rest of the app
 * gives skills.
 */
function Row({ spec, agent, active, onPick, onHover }) {
  return (
    <button
      type="button"
      role="option"
      aria-selected={active}
      // Pointer down, not click: a click fires after the editor has already
      // lost focus and the engine has thrown the selection away, so the
      // mention would be spliced in at an offset that no longer exists.
      onMouseDown={(event) => {
        event.preventDefault();
        onPick(spec);
      }}
      onMouseMove={onHover}
      className={cn(
        "flex h-9 w-full items-center gap-2.5 rounded-sm px-2 text-left outline-none",
        "text-[13px] text-foreground transition-colors duration-150 ease-out",
        active && "fill-menu"
      )}
    >
      {agent ? (
        <AgentAvatar agent={agent} size="xs" />
      ) : (
        <span className="flex size-5 shrink-0 items-center justify-center rounded-sm fill-secondary">
          <Icon
            name={spec.kind === "command" ? "Command" : "Sparkles"}
            className="size-3 text-muted-foreground"
          />
        </span>
      )}
      {/* A command reads the other way round from a name. `/compact` IS the
          thing - the word you are choosing and the word you will type - and
          what it does is the gloss; an agent or a skill has a name you know
          and a token you do not. So the two halves swap places rather than
          the row growing a third column nobody reads. */}
      {spec.kind === "command" ? (
        <>
          <span className="shrink-0 font-mono text-[12px]">
            {spec.raw}
            {spec.argHint ? <span className="text-muted-foreground"> {spec.argHint}</span> : null}
          </span>
          <span className="min-w-0 flex-1 truncate text-right text-[11px] text-muted-foreground">
            {spec.hint}
          </span>
        </>
      ) : (
        <>
          <span className="min-w-0 flex-1 truncate">{spec.label}</span>
          {/* The token gives way, not the name. It was `shrink-0`, and a long
              handle then truncated the thing you actually read the row for -
              "Inerti…" beside a complete `@inertia-devfolder-organizer`. */}
          <span className="min-w-0 shrink truncate font-mono text-[11px] text-muted-foreground">
            {spec.raw}
          </span>
        </>
      )}
    </button>
  );
}

export function MentionPalette({ palette, agents = [] }) {
  const { open, items, kind, index, setIndex, anchorRef, accept, close } = palette;

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
      }}
      anchorRef={anchorRef}
      side="top"
      align="start"
      offset={8}
      // The editor keeps focus, so an outside click is a click that has already
      // moved the caret; the palette closes on its own when the token does.
      dismissOnOutside={false}
      role="listbox"
      ariaLabel={kind === "agent" ? "Agents" : "Commands and skills"}
      className="w-72 max-h-72"
    >
      {items.map((spec, i) => (
        <Row
          key={spec.raw}
          spec={spec}
          agent={spec.kind === "agent" ? agents.find((one) => one.id === spec.id) : null}
          active={i === index}
          onHover={() => setIndex(i)}
          onPick={accept}
        />
      ))}
    </Popover>
  );
}
