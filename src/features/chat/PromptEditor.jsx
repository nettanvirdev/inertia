import React from "react";
import { cn } from "@/lib/utils";
import {
  caretOffset,
  domPillKey,
  renderText,
  serialize,
  setCaret,
  textPillKey,
} from "./mention-dom.js";

/*
 * PromptEditor - the composer's text surface.
 *
 * This replaces a `<textarea>`. A textarea holds characters and nothing else,
 * so it could colour a mention but never draw one - which is why an attached
 * thing had to appear twice, as text in the sentence and again as a chip in a
 * row above it. Here `@nova` IS the chip, in the sentence, where it was typed.
 *
 * The value is still a plain string; see mention-dom.js for why that matters
 * more than it sounds. What this component owns is narrow: render the string,
 * report edits back as a string, and say where the caret is so the palettes
 * can do their own work. It has no idea what an agent is.
 */

export const PromptEditor = React.forwardRef(function PromptEditor(
  {
    value,
    onChange,
    mentions,
    placeholder,
    className,
    disabled,
    onKeyDown,
    onCaretChange,
    onFocus,
    onBlur,
    onPasteFiles,
    elementRef,
    "aria-label": ariaLabel,
  },
  ref
) {
  const rootRef = React.useRef(null);
  const valueRef = React.useRef("");
  valueRef.current = String(value ?? "");
  /**
   * The last string this component put INTO the DOM or read OUT of it.
   *
   * An incoming value that differs from it came from outside and needs a
   * repaint; one that matches must be left alone, or every keystroke would
   * rebuild the DOM under the caret.
   *
   * `null` rather than `""` so the FIRST paint always happens. Starting it at
   * the empty string meant an editor mounted empty - which is every new
   * conversation - matched on the first pass and was never painted at all, so
   * the element had no text node and no sentinel in it and the caret had
   * nowhere legitimate to sit.
   */
  const syncedRef = React.useRef(null);
  const composingRef = React.useRef(false);
  /**
   * A caret position asked for before the value it refers to has arrived.
   *
   * Accepting a mention sets state and asks for a caret in the same tick, so
   * the position names an offset in text the DOM does not hold yet.
   */
  const pendingCaretRef = React.useRef(null);

  React.useImperativeHandle(
    ref,
    () => ({
      focus: () => {
        const root = rootRef.current;
        if (!root) return;
        root.focus();
        // Chromium chooses where the caret goes in a focused editable that has
        // no selection yet, and it does not always choose the start. The end
        // of the text - which is the start, when there is none - is chosen
        // for it, unless a selection is already in here, in which case it is
        // somebody's and is left alone.
        if (caretOffset(root) === null) setCaret(root, valueRef.current.length);
      },
      setCaret: (offset) => {
        const root = rootRef.current;
        if (!root) return;
        root.focus();
        // Applied twice on purpose: now, for the case where the value did not
        // change and no repaint is coming; and again after the repaint, for
        // the case where it did - because right now the DOM still holds the
        // old text, in which this offset means something else.
        pendingCaretRef.current = offset;
        setCaret(root, offset);
      },
      getCaret: () => (rootRef.current ? caretOffset(rootRef.current) : null),
    }),
    []
  );

  // Paint an externally-changed value. Layout effect, not effect: the repaint
  // and the caret restore have to land in the same frame or the caret visibly
  // jumps to the start and back.
  React.useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root || composingRef.current) return;
    if (value === syncedRef.current) return;

    const caret = caretOffset(root);
    renderText(root, value, mentions);
    syncedRef.current = value;

    // A position asked for before this repaint wins: it was worked out against
    // the text just painted, where `caret` refers to the text just thrown away.
    const wanted = pendingCaretRef.current ?? caret;
    pendingCaretRef.current = null;
    // Only restore the caret if it was in here to begin with - stealing focus
    // because a prop changed is its own bug.
    if (wanted !== null) setCaret(root, Math.min(wanted, value.length));
  }, [value, mentions]);

  // A change to the catalogue - an agent joining the conversation, skills
  // finishing loading - can turn text already typed into a pill. Same repaint,
  // different trigger: the value has not changed, so the effect above is quiet.
  React.useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root || composingRef.current) return;
    if (domPillKey(root) === textPillKey(value, mentions)) return;

    const caret = caretOffset(root);
    renderText(root, value, mentions);
    syncedRef.current = value;
    if (caret !== null) setCaret(root, caret);
  }, [value, mentions]);

  const emit = React.useCallback(() => {
    const root = rootRef.current;
    if (!root) return;

    // Select-all then cut leaves the element genuinely empty - no text node,
    // no sentinel, nothing for a caret to be inside - and the engine then
    // parks it against the placeholder pseudo-element instead of at the start
    // of the field.
    if (root.childNodes.length === 0) {
      renderText(root, "", mentions);
      setCaret(root, 0);
    }

    let next = serialize(root);

    // Deleting everything leaves the engine's own <br> behind, which reads
    // back as a lone newline - a value that is not empty and shows nothing. If there is
    // no text and no pill in here, there is nothing in here. A newline the
    // person typed lives in a text node and is kept.
    if (!root.textContent && !root.querySelector("[data-mention]")) next = "";

    // Typing the last character of a mention should turn it into a pill on the
    // spot, and deleting into one should turn it back into text. Both show up
    // as a mismatch between what the DOM has and what the text implies.
    if (domPillKey(root) !== textPillKey(next, mentions)) {
      const caret = caretOffset(root);
      renderText(root, next, mentions);
      if (caret !== null) setCaret(root, caret);
      next = serialize(root);
    }

    syncedRef.current = next;
    onChange(next);
    onCaretChange?.(next, caretOffset(root) ?? next.length);
  }, [mentions, onChange, onCaretChange]);

  /** Insert plain text at the caret, replacing any selection. */
  const insertText = React.useCallback(
    (text) => {
      const root = rootRef.current;
      const sel = window.getSelection();
      if (!root || !sel || sel.rangeCount === 0) return;

      const range = sel.getRangeAt(0);
      if (!root.contains(range.startContainer)) return;
      range.deleteContents();
      const node = document.createTextNode(text);
      range.insertNode(node);
      range.setStartAfter(node);
      range.collapse(true);
      sel.removeAllRanges();
      sel.addRange(range);
      emit();
    },
    [emit]
  );

  const handleKeyDown = (event) => {
    onKeyDown?.(event);
    if (event.defaultPrevented) return;

    if (event.key === "Enter") {
      // Newlines are ours to insert. Left to the engine, a contenteditable
      // answers Enter with a <div> or a <p> depending on which engine it is,
      // and the value would start carrying block structure the string has no
      // way to express.
      event.preventDefault();
      insertText("\n");
    }
  };

  /**
   * Put the SELECTION's raw text on the clipboard, tokens and all.
   *
   * Without this, copying a pill copies what the pill says: the engine's own
   * text/plain rendering is the label, so pasting it anywhere - including back
   * in here - produced a word instead of a mention.
   */
  const writeSelection = (event) => {
    const root = rootRef.current;
    const sel = window.getSelection();
    if (!root || !sel || sel.rangeCount === 0 || sel.isCollapsed) return null;

    const range = sel.getRangeAt(0);
    if (!root.contains(range.commonAncestorContainer)) return null;

    // A detached copy of the selected nodes through the same serialiser, so a
    // half-selected pill comes out whole or not at all, as it behaves on screen.
    const holder = document.createElement("div");
    holder.appendChild(range.cloneContents());
    event.clipboardData.setData("text/plain", serialize(holder));
    event.preventDefault();
    return range;
  };

  const reportCaret = () => {
    const root = rootRef.current;
    if (!root || !onCaretChange) return;
    const caret = caretOffset(root);
    if (caret !== null) onCaretChange(serialize(root), caret);
  };

  return (
    <div
      ref={(node) => {
        rootRef.current = node;
        if (elementRef) elementRef.current = node;
      }}
      role="textbox"
      aria-multiline="true"
      aria-label={ariaLabel}
      data-placeholder={placeholder}
      /* The placeholder's trigger. CSS cannot ask "is there any text in here",
         and the selectors that look like they could - :empty, :has(> br:only-child)
         - both get it wrong once the trailing sentinel <br> exists.
         Any character at all, the way a textarea does it - including a
         newline, because three Shift+Enters put the caret on line four and
         the placeholder was still sitting on line one. The one path that
         used to leave invisible whitespace behind, an emptied editable, is
         caught in emit() instead. */
      data-empty={String(value ?? "").length === 0 ? "true" : "false"}
      contentEditable={!disabled}
      suppressContentEditableWarning
      // Mentions are names and slugs, not prose - every agent name and skill
      // slug picks up a red squiggle otherwise.
      spellCheck={false}
      onInput={emit}
      onKeyDown={handleKeyDown}
      onKeyUp={reportCaret}
      onMouseUp={reportCaret}
      onCopy={writeSelection}
      onCut={(event) => {
        const range = writeSelection(event);
        if (!range) return;
        range.deleteContents();
        emit();
      }}
      onPaste={(event) => {
        if (event.clipboardData?.files?.length && onPasteFiles) {
          event.preventDefault();
          onPasteFiles([...event.clipboardData.files]);
          return;
        }
        // Plain text only: pasted markup arrives as elements the serialiser
        // has no representation for.
        event.preventDefault();
        insertText(event.clipboardData.getData("text/plain"));
      }}
      onFocus={onFocus}
      onBlur={onBlur}
      onCompositionStart={() => {
        composingRef.current = true;
      }}
      onCompositionEnd={() => {
        composingRef.current = false;
        emit();
      }}
      className={cn("prompt-editor", className)}
    />
  );
});
