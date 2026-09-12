/*
 * mention-dom - the text <-> DOM bridge under PromptEditor.
 *
 * The composer is a contenteditable, but its VALUE is still the plain string
 * mentions.js describes. That is the whole trick: the pills are a rendering of
 * the text rather than a new data model, so the draft that gets saved, the
 * message that gets sent and the history the main process reads are all the
 * same string they were when this was a textarea.
 *
 * These functions are deliberately plain DOM, not React. A contenteditable and
 * a React reconciler both want to own the same children, and when they
 * disagree the caret is what pays. React renders the shell; this renders the
 * contents, and only when the incoming string differs from what was last
 * serialised out of it.
 *
 * Nothing here can be tested - the suite runs in node with no document - which
 * is exactly why every decision that can be made in string space was made in
 * mentions.js instead.
 */

import { findSpecRanges } from "./mentions.js";

/** Marks the sentinel <br> that keeps a trailing newline reachable. */
const SENTINEL = "data-prompt-sentinel";

/* ── string -> DOM ─────────────────────────────────────────────────────── */

function pillNode(spec) {
  const el = document.createElement("span");
  // contentEditable=false is what makes the pill ATOMIC: the browser then
  // treats it as one object, so a single backspace removes the whole mention
  // rather than eating "nova" a letter at a time.
  el.contentEditable = "false";
  el.dataset.mention = spec.raw;
  el.className = "prompt-pill";
  el.setAttribute("data-kind", spec.kind);

  // The same face the header and the transcript draw, in the same order:
  // the picture if there is one, else the agent's icon, else its initials on
  // its own colour. `iconHtml` is the icon already rendered to markup by the
  // caller, because this is plain DOM and cannot mount a React component.
  if (spec.iconSrc) {
    const img = document.createElement("img");
    img.src = spec.iconSrc;
    img.alt = "";
    img.className = "prompt-pill-icon";
    // A picture that fails to load should cost the pill its icon, not leave a
    // broken-image glyph in the middle of the sentence.
    img.onerror = () => img.remove();
    el.appendChild(img);
  } else if (spec.iconHtml) {
    const glyph = document.createElement("span");
    glyph.className = "prompt-pill-glyph";
    glyph.innerHTML = spec.iconHtml;
    if (spec.tint) glyph.style.setProperty("--pill-tint", spec.tint);
    el.appendChild(glyph);
  } else if (spec.initials) {
    const badge = document.createElement("span");
    badge.className = "prompt-pill-initials";
    badge.textContent = spec.initials;
    if (spec.tint) badge.style.setProperty("--pill-tint", spec.tint);
    el.appendChild(badge);
  }

  const text = document.createElement("span");
  text.textContent = spec.label;
  el.appendChild(text);
  return el;
}

/**
 * Rebuild `root`'s children from `text`, drawing every known mention as a pill.
 *
 * Destructive by design - call it only when the value genuinely changed from
 * outside, or the caret is thrown to the start on every keystroke.
 */
export function renderText(root, text, specs) {
  root.replaceChildren();

  let cursor = 0;
  const pushText = (chunk) => {
    if (chunk) root.appendChild(document.createTextNode(chunk));
  };

  for (const { start, end, spec } of findSpecRanges(text, specs)) {
    pushText(text.slice(cursor, start));
    root.appendChild(pillNode(spec));
    cursor = end;
  }
  pushText(text.slice(cursor));

  // A text node ending in "\n" renders its break under `white-space: pre-wrap`,
  // but there is no position AFTER it for the caret to sit in - press Enter on
  // the last line and the caret appears not to move. A trailing <br> gives that
  // position somewhere to be. serialize() knows to ignore it.
  const br = document.createElement("br");
  br.setAttribute(SENTINEL, "");
  root.appendChild(br);
}

/* ── DOM -> string ─────────────────────────────────────────────────────── */

/** The text `root` currently holds, with pills back as their raw tokens. */
export function serialize(root) {
  let out = "";

  const walk = (node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      out += node.nodeValue ?? "";
      return;
    }
    if (!(node instanceof HTMLElement)) return;

    if (node.dataset.mention) {
      out += node.dataset.mention;
      return;
    }
    if (node.tagName === "BR") {
      // The sentinel is scaffolding, not content. So is the stray <br> some
      // engines append to an empty editable.
      if (!node.hasAttribute(SENTINEL)) out += "\n";
      return;
    }
    // A pasted or browser-generated block (<div>, <p>) starts a new line.
    const block = node.tagName === "DIV" || node.tagName === "P";
    if (block && out && !out.endsWith("\n")) out += "\n";
    node.childNodes.forEach(walk);
  };

  root.childNodes.forEach(walk);
  return out;
}

/** The pill list currently in the DOM, as a comparable key. */
export function domPillKey(root) {
  return Array.from(root.querySelectorAll("[data-mention]"))
    .map((el) => el.dataset.mention)
    .join(" ");
}

/** The pill list `text` should produce, as the same comparable key. */
export function textPillKey(text, specs) {
  return findSpecRanges(text, specs)
    .map((range) => range.spec.raw)
    .join(" ");
}

/* ── caret ─────────────────────────────────────────────────────────────── */

/** How many characters of `serialize(root)` precede `(node, offset)`. */
function offsetOf(root, node, nodeOffset) {
  let count = 0;
  let found = false;

  const walk = (current) => {
    if (found) return;

    if (current === node && current.nodeType === Node.TEXT_NODE) {
      count += nodeOffset;
      found = true;
      return;
    }
    if (current.nodeType === Node.TEXT_NODE) {
      count += (current.nodeValue ?? "").length;
      return;
    }
    if (!(current instanceof HTMLElement)) return;

    if (current.dataset.mention) {
      count += current.dataset.mention.length;
      return;
    }
    if (current.tagName === "BR") {
      if (!current.hasAttribute(SENTINEL)) count += 1;
      return;
    }
    // An element-anchored selection counts the children before `nodeOffset`.
    if (current === node) {
      const kids = Array.from(current.childNodes);
      for (let i = 0; i < nodeOffset && i < kids.length; i += 1) walk(kids[i]);
      found = true;
      return;
    }
    current.childNodes.forEach(walk);
  };

  walk(root);
  return count;
}

/** Where the caret is in the serialised text, or null if it isn't in here. */
export function caretOffset(root) {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return null;
  const range = sel.getRangeAt(0);
  if (!root.contains(range.startContainer)) return null;
  return offsetOf(root, range.startContainer, range.startOffset);
}

/** A collapsed Range at `target` characters into the serialised text. */
function rangeAt(root, target) {
  let count = 0;
  let placed = false;
  const range = document.createRange();

  const walk = (node) => {
    if (placed) return;

    if (node.nodeType === Node.TEXT_NODE) {
      const len = (node.nodeValue ?? "").length;
      if (count + len >= target) {
        range.setStart(node, Math.max(0, target - count));
        placed = true;
        return;
      }
      count += len;
      return;
    }
    if (!(node instanceof HTMLElement)) return;

    if (node.dataset.mention) {
      const len = node.dataset.mention.length;
      if (count + len >= target) {
        range.setStartAfter(node);
        placed = true;
        return;
      }
      count += len;
      return;
    }
    if (node.tagName === "BR") {
      if (node.hasAttribute(SENTINEL)) return;
      if (count + 1 >= target) {
        range.setStartAfter(node);
        placed = true;
        return;
      }
      count += 1;
      return;
    }
    node.childNodes.forEach(walk);
  };

  walk(root);

  if (!placed) {
    range.selectNodeContents(root);
    range.collapse(false);
  } else {
    range.collapse(true);
  }
  return range;
}

/**
 * Put the caret at `target` characters into the serialised text.
 *
 * An offset landing INSIDE a pill snaps to just after it: a pill is one
 * object, so there is no such thing as being three characters into it.
 */
export function setCaret(root, target) {
  const sel = window.getSelection();
  if (!sel) return;
  sel.removeAllRanges();
  sel.addRange(rangeAt(root, target));
}

/**
 * Where a character offset is on screen, measured with a real element.
 *
 * This is what the mention list hangs off, and it is deliberately NOT the
 * live selection. The selection is only where we think it is while the editor
 * has focus, and there are several ways for it not to at the moment the list
 * is placed: the `+` menu types the `@` for you and then hands focus back to
 * its own button; a click lands between the keystroke and the measurement; a
 * blur and refocus reorders things. Each of those put the list somewhere the
 * cursor was not. The token's offset in the string is known regardless, so
 * the measurement goes through that.
 *
 * And it goes through an element rather than a Range: a zero-width marker is
 * put at the offset, measured with the same `getBoundingClientRect` every
 * other anchored layer in the app uses, and taken out again. Collapsed Range
 * rects are the thing that reports 0x0 more often than not in Chromium; an
 * element never does. Splitting a text node moves the live selection, so the
 * caret is put back afterwards if it was in here.
 */
export function offsetRect(root, offset) {
  if (!root) return null;
  const restore = caretOffset(root);
  const range = rangeAt(root, offset);
  const marker = document.createElement("span");
  marker.setAttribute("data-prompt-marker", "");
  // A zero-width space gives an otherwise empty inline a line box to measure.
  marker.textContent = "\u200b";
  range.insertNode(marker);
  const measured = marker.getBoundingClientRect();
  const parent = marker.parentNode;
  marker.remove();
  // Inserting into a text node split it in two; put it back as one.
  parent?.normalize?.();
  if (restore !== null) setCaret(root, restore);
  if (measured.height <= 0) return null;
  return new DOMRect(measured.left, measured.top, 0, measured.height);
}

/**
 * Where the cursor actually is on screen, or null if it isn't in `root`.
 *
 * The palette hangs off this rather than off the field. A list pinned to the
 * corner of the composer is fine while the composer is one line tall and wrong
 * the moment it is not: type six lines, press `@` on the last, and the choices
 * appear at the top of the box a long way from the cursor, reading as an
 * unrelated panel.
 */
export function caretRect(root) {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return null;
  const range = sel.getRangeAt(0);
  if (!root.contains(range.startContainer)) return null;

  // The last rect, not the bounding box. An offset that falls exactly on a
  // soft wrap has TWO screen positions - the end of the line it wrapped out
  // of and the start of the line it wrapped into - and the union of them is
  // on neither.
  const rects = range.getClientRects();
  if (rects.length > 0 && rects[rects.length - 1].height > 0) {
    return rects[rects.length - 1];
  }

  const direct = range.getBoundingClientRect();
  if (direct.height > 0) return direct;

  // A collapsed Range measures 0x0 in Chromium more often than not, so measure
  // the character BEFORE the caret and use its trailing edge. That character
  // is on the caret's own visual line by construction.
  const { startContainer: node, startOffset } = range;
  if (node.nodeType !== Node.TEXT_NODE) return root.getBoundingClientRect();

  const probe = document.createRange();
  if (startOffset > 0) {
    probe.setStart(node, startOffset - 1);
    probe.setEnd(node, startOffset);
    const r = probe.getBoundingClientRect();
    if (r.height > 0) return new DOMRect(r.right, r.top, 0, r.height);
  }
  if ((node.nodeValue ?? "").length > startOffset) {
    probe.setStart(node, startOffset);
    probe.setEnd(node, startOffset + 1);
    const r = probe.getBoundingClientRect();
    if (r.height > 0) return new DOMRect(r.left, r.top, 0, r.height);
  }
  return root.getBoundingClientRect();
}
