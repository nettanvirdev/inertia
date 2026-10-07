/**
 * Block-level markdown, line by line, into a flat list of block objects.
 *
 * Pure data in, pure data out - no React here, so the whole grammar is
 * testable directly and the renderer stays a thin mapping from block to
 * element.
 *
 * The shape is deliberately small:
 *   { type: "paragraph", text }
 *   { type: "heading", level, text }
 *   { type: "code", lang, code, closed }
 *   { type: "quote", blocks }
 *   { type: "list", ordered, start, tight, items: [{ checked, blocks }] }
 *   { type: "table", head, align, rows, partial }
 *   { type: "math", value }
 *   { type: "footnote", id, blocks }
 *   { type: "hr" }
 *
 * Nested constructs (list item bodies, quote bodies) hold their own `blocks`
 * array produced by the same function, so nesting costs no extra grammar.
 *
 * ── Streaming ──────────────────────────────────────────────────────────────
 * Everything here is written so that a block, once it has taken a form, keeps
 * that form as more characters arrive. Two cases needed explicit handling:
 *
 *  · An unterminated fence closes at end of input (which is also what
 *    CommonMark does at EOF), so a code block is a code block from the moment
 *    the opening fence lands - it never appears as raw text first.
 *  · A table whose alignment row has not been typed yet would otherwise spend
 *    several frames as a paragraph full of pipes. While streaming, a trailing
 *    header-shaped line is emitted as a table with `partial: true` instead.
 *    Rendering it as a table in progress rather than holding it back was the
 *    choice: holding it back makes the text visibly stall and then jump, and
 *    the header cells are already known, so the columns do not move when the
 *    alignment row finally arrives.
 */

const FENCE = /^ {0,3}(```+|~~~+)[ \t]*([^`\s]*)[^`]*$/;
// A formula on its own, in either of the two spellings models use.
const MATH_OPEN = /^ {0,3}(\$\$|\\\[)(.*)$/;
// `[^1]: the note`, a footnote definition. Its continuation lines are the
// indented ones under it, the way a list item's are.
const FOOTNOTE = /^ {0,3}\[\^([^\]\s]+)\]:[ \t]*(.*)$/;
const HR = /^ {0,3}(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$/;
const ATX = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*#*[ \t]*$/;
const QUOTE = /^ {0,3}>/;
const ITEM = /^([ \t]*)([-+*]|\d{1,9}[.)])([ \t]+|$)/;
const SETEXT = /^ {0,3}(=+|-{3,})[ \t]*$/;
const ALIGN_ROW = /^[ \t]*\|?[ \t]*:?-+:?[ \t]*(\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$/;
// a line that could still grow into an alignment row: `|--` , `| :-` , `|`
const ALIGN_PREFIX = /^[ \t]*[|:\- \t]*$/;

const MAX_DEPTH = 6;

function leadingSpaces(line) {
  const match = /^[ \t]*/.exec(line);
  let n = 0;
  for (const ch of match[0]) n += ch === "\t" ? 4 : 1;
  return n;
}

function isBlank(line) {
  return !line.trim();
}

/** Anything that interrupts a lazy paragraph continuation. */
function startsBlock(line) {
  return FENCE.test(line) || HR.test(line) || ATX.test(line) || QUOTE.test(line) || ITEM.test(line);
}

function splitRow(line) {
  let text = line.trim();
  if (text.startsWith("|")) text = text.slice(1);
  if (text.endsWith("|") && !text.endsWith("\\|")) text = text.slice(0, -1);
  const cells = [];
  let cell = "";
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === "\\" && text[i + 1] === "|") {
      cell += "|";
      i += 1;
    } else if (ch === "|") {
      cells.push(cell.trim());
      cell = "";
    } else {
      cell += ch;
    }
  }
  cells.push(cell.trim());
  return cells;
}

function parseAlign(line) {
  if (!line || !ALIGN_ROW.test(line) || !line.includes("-")) return null;
  return splitRow(line).map((cell) => {
    const left = cell.startsWith(":");
    const right = cell.endsWith(":");
    if (left && right) return "center";
    if (right) return "right";
    if (left) return "left";
    return null;
  });
}

function fitRow(cells, width) {
  const row = cells.slice(0, width);
  while (row.length < width) row.push("");
  return row;
}

export function parseBlocks(source, options = {}) {
  const { streaming = false, depth = 0 } = options;
  const lines = String(source ?? "")
    .replace(/\r\n?/g, "\n")
    .split("\n");
  const blocks = [];
  let paragraph = [];
  let i = 0;

  const flush = () => {
    if (paragraph.length) {
      blocks.push({ type: "paragraph", text: paragraph.join("\n") });
      paragraph = [];
    }
  };

  while (i < lines.length) {
    const line = lines[i];

    // ── fenced code ────────────────────────────────────────────────────────
    const fence = FENCE.exec(line);
    if (fence) {
      flush();
      const marker = fence[1][0];
      const width = fence[1].length;
      const body = [];
      const indent = leadingSpaces(line);
      i += 1;
      let closed = false;
      while (i < lines.length) {
        const candidate = lines[i];
        const closer = /^ {0,3}(`{3,}|~{3,})[ \t]*$/.exec(candidate);
        if (closer && closer[1][0] === marker && closer[1].length >= width) {
          closed = true;
          i += 1;
          break;
        }
        // a fence opened inside a list item carries the item's indentation
        body.push(indent ? candidate.replace(new RegExp(`^ {0,${indent}}`), "") : candidate);
        i += 1;
      }
      blocks.push({ type: "code", lang: fence[2] || "", code: body.join("\n"), closed });
      continue;
    }

    // -- a footnote's text --------------------------------------------------
    // Collected as its own block so the renderer can put every note together
    // at the foot of the message. Left in the stream, they read as a stray
    // line saying `[^1]: ...` in the middle of the prose, which is what a
    // reader was seeing.
    const footnote = FOOTNOTE.exec(line);
    if (footnote && !paragraph.length) {
      flush();
      const body = [footnote[2]];
      i += 1;
      while (i < lines.length) {
        const candidate = lines[i];
        if (isBlank(candidate)) break;
        if (FOOTNOTE.test(candidate) || startsBlock(candidate)) break;
        body.push(candidate.trim());
        i += 1;
      }
      blocks.push({
        type: "footnote",
        id: footnote[1],
        blocks:
          depth < MAX_DEPTH
            ? parseBlocks(body.join("\n"), { streaming, depth: depth + 1 })
            : [{ type: "paragraph", text: body.join("\n") }],
      });
      continue;
    }

    // -- display maths ------------------------------------------------------
    // Closed only: a formula half typed would be laid out wrong and then jump,
    // so while it streams it stays the text it is.
    const math = MATH_OPEN.exec(line);
    if (math && !paragraph.length) {
      const closer = math[1] === "$$" ? "$$" : "\\]";
      const first = math[2].trim();
      if (first.endsWith(closer)) {
        flush();
        blocks.push({ type: "math", value: first.slice(0, -closer.length).trim() });
        i += 1;
        continue;
      }
      const body = first ? [first] : [];
      let closed = false;
      let j = i + 1;
      while (j < lines.length) {
        const candidate = lines[j].trim();
        if (candidate.endsWith(closer)) {
          const tail = candidate.slice(0, -closer.length).trim();
          if (tail) body.push(tail);
          closed = true;
          j += 1;
          break;
        }
        body.push(lines[j]);
        j += 1;
      }
      if (closed || !streaming) {
        flush();
        blocks.push({ type: "math", value: body.join("\n").trim() });
        i = j;
        continue;
      }
    }

    if (isBlank(line)) {
      flush();
      i += 1;
      continue;
    }

    // ── setext heading ─────────────────────────────────────────────────────
    // Only `===` and `---` (three or more), so a lone `-` still reads as an
    // empty list bullet rather than silently promoting the line above it.
    if (paragraph.length && SETEXT.test(line)) {
      const level = line.trim().startsWith("=") ? 1 : 2;
      blocks.push({ type: "heading", level, text: paragraph.join("\n") });
      paragraph = [];
      i += 1;
      continue;
    }

    if (HR.test(line)) {
      flush();
      blocks.push({ type: "hr" });
      i += 1;
      continue;
    }

    const atx = ATX.exec(line);
    if (atx) {
      flush();
      blocks.push({ type: "heading", level: atx[1].length, text: (atx[2] ?? "").trim() });
      i += 1;
      continue;
    }

    // ── blockquote ─────────────────────────────────────────────────────────
    if (QUOTE.test(line)) {
      flush();
      const body = [];
      while (i < lines.length) {
        const candidate = lines[i];
        if (QUOTE.test(candidate)) {
          body.push(candidate.replace(/^ {0,3}>[ \t]?/, ""));
          i += 1;
          continue;
        }
        // a blank line, or any new block, ends the quote; plain prose is a
        // lazy continuation of the quote's last paragraph
        if (isBlank(candidate) || startsBlock(candidate)) break;
        body.push(candidate);
        i += 1;
      }
      blocks.push({
        type: "quote",
        blocks:
          depth < MAX_DEPTH
            ? parseBlocks(body.join("\n"), { streaming, depth: depth + 1 })
            : [{ type: "paragraph", text: body.join("\n") }],
      });
      continue;
    }

    // ── table ──────────────────────────────────────────────────────────────
    if (line.includes("|")) {
      const head = splitRow(line);
      const align = parseAlign(lines[i + 1]);
      // The column counts must agree, otherwise a setext heading whose text
      // happens to contain a pipe would be read as a one-column table.
      if (align && align.length === head.length) {
        flush();
        const width = head.length;
        const rows = [];
        i += 2;
        while (i < lines.length) {
          const candidate = lines[i];
          if (isBlank(candidate) || !candidate.includes("|") || startsBlock(candidate)) break;
          rows.push(fitRow(splitRow(candidate), width));
          i += 1;
        }
        blocks.push({
          type: "table",
          head: fitRow(head, width),
          align: fitRow(align, width).map((a) => a || null),
          rows,
          partial: false,
        });
        continue;
      }

      // Header typed, alignment row still arriving. Only at the very end of
      // the stream, so a sentence containing a pipe mid-message is untouched.
      const next = lines[i + 1];
      const isTail =
        i === lines.length - 1 || (i === lines.length - 2 && ALIGN_PREFIX.test(next ?? ""));
      // Table-shaped means fenced by pipes, or already followed by the start
      // of an alignment row. A sentence that merely contains a pipe is prose.
      const leads = line.trim().startsWith("|");
      // A line that opens with a pipe is a table row and nothing else, even
      // with one cell typed so far; the looser shapes need two cells before
      // they are worth believing.
      const shaped =
        leads ||
        ((line.trim().endsWith("|") ||
          (next !== undefined && ALIGN_PREFIX.test(next) && next.includes("-"))) &&
          head.length > 1);
      if (streaming && isTail && shaped && !paragraph.length) {
        blocks.push({ type: "table", head, align: head.map(() => null), rows: [], partial: true });
        i = lines.length;
        continue;
      }
    }

    // ── list ───────────────────────────────────────────────────────────────
    const item = ITEM.exec(line);
    if (item) {
      flush();
      const ordered = /\d/.test(item[2]);
      const baseIndent = leadingSpaces(item[1]);
      const start = ordered ? Number.parseInt(item[2], 10) : 1;
      const items = [];
      let loose = false;

      while (i < lines.length) {
        const head = ITEM.exec(lines[i]);
        if (!head) break;
        if (leadingSpaces(head[1]) > baseIndent + 3) break;
        if (/\d/.test(head[2]) !== ordered) break;

        const contentIndent = leadingSpaces(head[1]) + head[2].length + Math.max(head[3].length, 1);
        const body = [lines[i].slice(Math.min(contentIndent, lines[i].length))];
        i += 1;
        let blankPending = false;

        while (i < lines.length) {
          const candidate = lines[i];
          if (isBlank(candidate)) {
            blankPending = true;
            body.push("");
            i += 1;
            continue;
          }
          if (leadingSpaces(candidate) >= contentIndent) {
            // indented content after a blank line is what makes a list loose
            if (blankPending) loose = true;
            blankPending = false;
            body.push(candidate.slice(contentIndent));
            i += 1;
            continue;
          }
          if (blankPending || startsBlock(candidate)) break;
          body.push(candidate); // lazy paragraph continuation
          i += 1;
        }

        while (body.length && isBlank(body[body.length - 1])) body.pop();
        // a blank line only loosens the list if another item follows it
        if (blankPending && i < lines.length && ITEM.test(lines[i])) loose = true;

        const itemBlocks =
          depth < MAX_DEPTH
            ? parseBlocks(body.join("\n"), { streaming, depth: depth + 1 })
            : [{ type: "paragraph", text: body.join("\n") }];

        let checked = null;
        const first = itemBlocks[0];
        if (first && first.type === "paragraph") {
          const task = /^\[([ xX])\](?:[ \t]+|$)/.exec(first.text);
          if (task) {
            checked = task[1] !== " ";
            itemBlocks[0] = { type: "paragraph", text: first.text.slice(task[0].length) };
          }
        }
        items.push({ checked, blocks: itemBlocks });
      }

      blocks.push({ type: "list", ordered, start, tight: !loose, items });
      continue;
    }

    // ── indented code ──────────────────────────────────────────────────────
    // Only when no paragraph is open, otherwise a wrapped sentence indented by
    // the model would turn into a code block halfway through.
    if (!paragraph.length && leadingSpaces(line) >= 4) {
      const body = [];
      while (i < lines.length && (isBlank(lines[i]) || leadingSpaces(lines[i]) >= 4)) {
        body.push(lines[i].replace(/^ {1,4}|^\t/, ""));
        i += 1;
      }
      while (body.length && isBlank(body[body.length - 1])) body.pop();
      blocks.push({ type: "code", lang: "", code: body.join("\n"), closed: true });
      continue;
    }

    paragraph.push(line);
    i += 1;
  }

  flush();
  return blocks;
}
