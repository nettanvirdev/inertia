/**
 * Inline markdown, scanned left to right into a small node tree.
 *
 * The scanner never emits a construct it has not seen the end of. That single
 * rule is what makes streaming safe: a half-typed `[label](htt` or a trailing
 * `**bo` is emitted as the literal characters that have arrived so far, so the
 * text stays put and simply changes weight the instant the closer lands. A
 * regex-based parser would either swallow the rest of the message hunting for
 * a closer or flip the whole tail between forms on every token.
 *
 * Nodes are plain objects so the parser can be tested without React:
 *   { type: "text", value }
 *   { type: "strong" | "em" | "del", children }
 *   { type: "code", value }
 *   { type: "link", href, children }
 *   { type: "image", src, alt }
 *   { type: "math", value }
 *   { type: "html", tag, children }
 *   { type: "footref", id }
 *   { type: "mention", handle }
 *   { type: "break" }
 */

const ESCAPABLE = "\\`*_{}[]()#+-.!|~<>\"'";
const AUTOLINK = /^(?:https?:\/\/|www\.)[^\s<>`]+/i;

/**
 * The end of a `$...$` formula, or -1.
 *
 * Two rules keep prices out of the maths. The content may not begin or end
 * with a space, which is what separates `$x + 1$` from "$5 and change $7"; and
 * content that is only a number is a price, because nobody writes `$42$`
 * meaning the integer.
 */
function findMathEnd(src, start) {
  if (/\s/.test(src[start + 1] ?? " ")) return -1;
  for (let i = start + 1; i < src.length; i += 1) {
    if (src[i] === "\\") {
      i += 1;
      continue;
    }
    if (src[i] === "\n") return -1;
    if (src[i] !== "$") continue;
    const value = src.slice(start + 1, i);
    if (!value.trim() || /\s$/.test(value)) return -1;
    if (/^[\d.,]+$/.test(value)) return -1;
    return i;
  }
  return -1;
}

function isAlnum(ch) {
  return !!ch && /[0-9A-Za-z]/.test(ch);
}

/** Runs of one character, used for backtick and emphasis delimiters. */
function runLength(src, index) {
  const ch = src[index];
  let n = 0;
  while (src[index + n] === ch) n += 1;
  return n;
}

/**
 * Finds a closing delimiter run of exactly `len`, skipping escapes and code
 * spans so `**a `b**` c**` closes where a reader expects. Returns -1 when the
 * closer has not arrived yet, which is the common case mid-stream.
 */
function findCloser(src, from, marker, len) {
  let i = from;
  while (i < src.length) {
    const ch = src[i];
    if (ch === "\\") {
      i += 2;
      continue;
    }
    if (ch === "`") {
      const n = runLength(src, i);
      const end = findCodeSpanEnd(src, i, n);
      i = end === -1 ? i + n : end + n;
      continue;
    }
    if (ch === marker) {
      const n = runLength(src, i);
      // a closer may not sit against whitespace on its left
      if (n >= len && !/\s/.test(src[i - 1] ?? "")) {
        if (marker !== "_" || !isAlnum(src[i + n])) return i;
      }
      i += n;
      continue;
    }
    i += 1;
  }
  return -1;
}

/** Index of the opening backtick of the closing run, or -1. */
function findCodeSpanEnd(src, start, len) {
  let i = start + len;
  while (i < src.length) {
    if (src[i] === "`") {
      const n = runLength(src, i);
      if (n === len) return i;
      i += n;
      continue;
    }
    i += 1;
  }
  return -1;
}

/** Balanced `]` for a link label, tolerating nesting and escapes. */
function findLabelEnd(src, start) {
  let depth = 0;
  for (let i = start; i < src.length; i += 1) {
    const ch = src[i];
    if (ch === "\\") {
      i += 1;
      continue;
    }
    if (ch === "[") depth += 1;
    else if (ch === "]") {
      if (depth === 0) return i;
      depth -= 1;
    }
  }
  return -1;
}

/** Balanced `)` for a link destination. */
function findDestEnd(src, start) {
  let depth = 0;
  for (let i = start; i < src.length; i += 1) {
    const ch = src[i];
    if (ch === "\\") {
      i += 1;
      continue;
    }
    if (ch === "\n") return -1;
    if (ch === "(") depth += 1;
    else if (ch === ")") {
      if (depth === 0) return i;
      depth -= 1;
    }
  }
  return -1;
}

/** `<https://x>` and a title after the destination are both stripped here. */
function cleanDestination(raw) {
  let dest = raw.trim();
  const title = /\s+["'(].*$/.exec(dest);
  if (title) dest = dest.slice(0, title.index);
  if (dest.startsWith("<") && dest.endsWith(">")) dest = dest.slice(1, -1);
  return dest;
}

/**
 * A bare URL swallows the sentence's punctuation otherwise: "see https://x.dev."
 * should not link the full stop, and a trailing `)` only belongs to the URL if
 * the URL opened it.
 */
function trimUrlTail(url) {
  let out = url;
  for (;;) {
    const last = out[out.length - 1];
    if (!last) break;
    if (".,;:!?'\"".includes(last)) {
      out = out.slice(0, -1);
      continue;
    }
    if (last === ")") {
      const opens = (out.match(/\(/g) || []).length;
      const closes = (out.match(/\)/g) || []).length;
      if (closes > opens) {
        out = out.slice(0, -1);
        continue;
      }
    }
    break;
  }
  return out;
}


/**
 * The small amount of HTML a model actually writes in a message.
 *
 * Models mix markup into markdown constantly, and the honest reasons to have
 * refused it - markup from a model must never become markup on the page - are
 * about `dangerouslySetInnerHTML`, not about the tags. So the tags are read
 * into the same node tree everything else produces, attribute by attribute,
 * and rendered as React elements. Nothing is passed through: a tag not on this
 * list stays the literal characters it was written as, which is exactly what
 * happened to all of them before.
 *
 * `<img>` is the one that matters. A model handed a URL writes
 * `<img src="..." width="600"/>` about as often as it writes `![](...)`, and a
 * reader who sees the raw tag has been shown the source of a picture instead
 * of the picture.
 */
const VOID_TAGS = { br: "break", img: "image", wbr: "break" };

const INLINE_TAGS = new Set([
  "b", "strong", "i", "em", "u", "s", "del", "strike", "code", "kbd", "mark",
  "small", "sub", "sup", "span", "a", "abbr", "cite", "q", "big", "var", "samp",
]);

/** The node an inline tag becomes, so the renderer has no HTML of its own. */
const TAG_NODE = {
  b: "strong", strong: "strong",
  i: "em", em: "em", var: "em", cite: "em",
  s: "del", del: "del", strike: "del",
};

function readAttributes(text) {
  const out = {};
  const attribute = /([A-Za-z_:][-A-Za-z0-9_:.]*)(?:\s*=\s*("[^"]*"|'[^']*'|[^\s"'>]+))?/g;
  let match;
  while ((match = attribute.exec(text))) {
    const value = match[2] ?? "";
    out[match[1].toLowerCase()] = value.replace(/^["']|["']$/g, "");
  }
  return out;
}

/**
 * An HTML tag at `start`, or null.
 *
 * Returns null for anything unfinished as well as anything unknown, which is
 * what keeps a half-typed `<img src="htt` from flickering: the characters stay
 * text until the whole tag has arrived.
 */
function readTag(src, start) {
  const open = /^<\s*([A-Za-z][A-Za-z0-9]*)((?:[^<>"']|"[^"]*"|'[^']*')*)>/.exec(src.slice(start));
  if (!open) return null;
  const name = open[1].toLowerCase();
  const raw = open[2] ?? "";
  const selfClosing = /\/\s*$/.test(raw);
  const attributes = readAttributes(raw.replace(/\/\s*$/, ""));
  const after = start + open[0].length;

  if (VOID_TAGS[name]) {
    if (name === "img") {
      const src_ = attributes.src ?? attributes.srcset?.split(/[ ,]/)[0] ?? "";
      if (!src_) return { node: null, next: after };
      const width = Number.parseInt(attributes.width, 10);
      return {
        node: {
          type: "image",
          src: src_,
          alt: attributes.alt ?? attributes.title ?? "",
          ...(Number.isFinite(width) && width > 0 ? { width } : {}),
        },
        next: after,
      };
    }
    return { node: { type: "break" }, next: after };
  }

  if (!INLINE_TAGS.has(name)) return null;
  if (selfClosing) return { node: null, next: after };

  // The matching close, allowing the same tag to nest once - `<b>a<b>b</b></b>`
  // is rare but a scanner that takes the first `</b>` gets it visibly wrong.
  let depth = 0;
  let at = after;
  let end = -1;
  const opener = new RegExp(`<\\s*${name}\\b`, "i");
  const closer = new RegExp(`<\\s*/\\s*${name}\\s*>`, "i");
  while (at < src.length) {
    const rest = src.slice(at);
    const nextOpen = rest.search(opener);
    const nextClose = rest.search(closer);
    if (nextClose === -1) break;
    if (nextOpen !== -1 && nextOpen < nextClose) {
      depth += 1;
      at += nextOpen + 1;
      continue;
    }
    if (depth === 0) {
      end = at + nextClose;
      break;
    }
    depth -= 1;
    at += nextClose + 1;
  }
  if (end === -1) return null; // not finished arriving

  const inner = src.slice(after, end);
  const next = end + closer.exec(src.slice(end))[0].length;

  if (name === "code") return { node: { type: "code", value: inner }, next };
  if (name === "a") {
    const href = attributes.href ?? "";
    if (!href) return { node: { type: "html", tag: "span", children: parseInline(inner) }, next };
    return { node: { type: "link", href, children: parseInline(inner) }, next };
  }
  const mapped = TAG_NODE[name];
  if (mapped) return { node: { type: mapped, children: parseInline(inner) }, next };
  return { node: { type: "html", tag: name, children: parseInline(inner) }, next };
}

/**
 * Prose, with the model's dashes taken out.
 *
 * Models reach for the em dash constantly - three to a paragraph, where a
 * person would have used a comma, a colon or a full stop - and on screen it
 * reads as exactly what it is. The em dash becomes a spaced hyphen, which is
 * how the same pause is typed by hand; the en dash becomes a plain one, so a
 * range still reads as a range. Only prose: this is applied to text nodes
 * alone, so a dash inside code, a link's address or a path is left as written.
 */
function prose(value) {
  return value.replace(/\s*—\s*/g, " - ").replace(/–/g, "-");
}

export function parseInline(source) {
  const src = String(source ?? "");
  const nodes = [];
  let buffer = "";

  const push = (node) => {
    if (buffer) {
      nodes.push({ type: "text", value: prose(buffer) });
      buffer = "";
    }
    nodes.push(node);
  };

  let i = 0;
  while (i < src.length) {
    const ch = src[i];

    // Maths first, because `\(` would otherwise be eaten as an escaped
    // bracket by the rule below it.
    if (ch === "\\" && src[i + 1] === "(") {
      const end = src.indexOf("\\)", i + 2);
      if (end !== -1) {
        push({ type: "math", value: src.slice(i + 2, end) });
        i = end + 2;
        continue;
      }
    }

    if (ch === "$" && src[i + 1] !== "$") {
      const end = findMathEnd(src, i);
      if (end !== -1) {
        push({ type: "math", value: src.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }

    if (ch === "\\") {
      const next = src[i + 1];
      // a backslash at the very end is still being typed - keep it literal
      if (next && ESCAPABLE.includes(next)) {
        buffer += next;
        i += 2;
        continue;
      }
      buffer += ch;
      i += 1;
      continue;
    }

    if (ch === "\n") {
      push({ type: "break" });
      i += 1;
      continue;
    }

    // `@handle`: a mention of an agent, the way the composer writes one and
    // the agents write each other. Only at a word boundary, so an email
    // address is left as an address; only the hyphenated word, so trailing
    // punctuation stays outside it. Which agent it is, or whether it is one at
    // all, is decided where it is drawn - the parser knows no names.
    if (ch === "@") {
      const before = i === 0 ? "" : src[i - 1];
      const match = /^@([\p{L}\p{N}][\p{L}\p{N}-]*)/u.exec(src.slice(i));
      if (match && !/[\p{L}\p{N}]/u.test(before)) {
        const handle = match[1].replace(/-+$/, "");
        push({ type: "mention", handle });
        i += 1 + handle.length;
        continue;
      }
    }

    if (ch === "`") {
      const len = runLength(src, i);
      const end = findCodeSpanEnd(src, i, len);
      if (end !== -1) {
        let value = src.slice(i + len, end);
        // CommonMark strips one padding space each side, which is how a span
        // can hold a literal backtick: `` ` ``
        if (value.length > 2 && value.startsWith(" ") && value.endsWith(" ") && value.trim())
          value = value.slice(1, -1);
        push({ type: "code", value });
        i = end + len;
        continue;
      }
      buffer += src.slice(i, i + len);
      i += len;
      continue;
    }

    if (ch === "!" && src[i + 1] === "[") {
      const labelEnd = findLabelEnd(src, i + 2);
      if (labelEnd !== -1 && src[labelEnd + 1] === "(") {
        const destEnd = findDestEnd(src, labelEnd + 2);
        if (destEnd !== -1) {
          push({
            type: "image",
            alt: src.slice(i + 2, labelEnd),
            src: cleanDestination(src.slice(labelEnd + 2, destEnd)),
          });
          i = destEnd + 1;
          continue;
        }
      }
      buffer += ch;
      i += 1;
      continue;
    }

    if (ch === "<") {
      const tag = readTag(src, i);
      if (tag) {
        if (tag.node) push(tag.node);
        i = tag.next;
        continue;
      }
    }

    // `[^1]`, a footnote reference. Checked before the link rule, which would
    // otherwise read `[^1]` as a label with no destination and leave it alone
    // as text - which is what a reader was seeing.
    if (ch === "[" && src[i + 1] === "^") {
      const close = src.indexOf("]", i + 2);
      const id = close === -1 ? "" : src.slice(i + 2, close);
      if (id && !/[\s[\]]/.test(id)) {
        push({ type: "footref", id });
        i = close + 1;
        continue;
      }
    }

    if (ch === "[") {
      const labelEnd = findLabelEnd(src, i + 1);
      if (labelEnd !== -1 && src[labelEnd + 1] === "(") {
        const destEnd = findDestEnd(src, labelEnd + 2);
        if (destEnd !== -1) {
          push({
            type: "link",
            href: cleanDestination(src.slice(labelEnd + 2, destEnd)),
            children: parseInline(src.slice(i + 1, labelEnd)),
          });
          i = destEnd + 1;
          continue;
        }
      }
      buffer += ch;
      i += 1;
      continue;
    }

    if (ch === "<") {
      const close = src.indexOf(">", i + 1);
      if (close !== -1) {
        const inner = src.slice(i + 1, close);
        if (/^(?:https?:\/\/|mailto:)[^\s<>]+$/i.test(inner)) {
          push({ type: "link", href: inner, children: [{ type: "text", value: inner }] });
          i = close + 1;
          continue;
        }
      }
      // anything else in angle brackets is literal text, never markup
      buffer += ch;
      i += 1;
      continue;
    }

    if (ch === "~" && src[i + 1] === "~") {
      const end = findCloser(src, i + 2, "~", 2);
      if (end !== -1 && end > i + 2) {
        push({ type: "del", children: parseInline(src.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
      buffer += "~~";
      i += 2;
      continue;
    }

    if (ch === "*" || ch === "_") {
      const len = Math.min(runLength(src, i), 3);
      const opensWord = ch === "_" && isAlnum(src[i - 1]);
      const nextChar = src[i + len];
      // an opener may not sit against whitespace on its right, and an
      // underscore inside a word (snake_case) never opens emphasis
      if (nextChar && !/\s/.test(nextChar) && !opensWord) {
        const end = findCloser(src, i + len, ch, len);
        if (end !== -1) {
          const inner = parseInline(src.slice(i + len, end));
          if (len === 3) push({ type: "strong", children: [{ type: "em", children: inner }] });
          else if (len === 2) push({ type: "strong", children: inner });
          else push({ type: "em", children: inner });
          i = end + len;
          continue;
        }
      }
      buffer += src.slice(i, i + len);
      i += len;
      continue;
    }

    if ((ch === "h" || ch === "H" || ch === "w" || ch === "W") && !isAlnum(src[i - 1])) {
      const match = AUTOLINK.exec(src.slice(i));
      if (match) {
        const url = trimUrlTail(match[0]);
        const href = url.toLowerCase().startsWith("www.") ? `https://${url}` : url;
        push({ type: "link", href, children: [{ type: "text", value: url }] });
        i += url.length;
        continue;
      }
    }

    buffer += ch;
    i += 1;
  }

  if (buffer) nodes.push({ type: "text", value: prose(buffer) });
  return nodes;
}
