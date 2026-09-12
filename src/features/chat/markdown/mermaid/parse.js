/**
 * Mermaid, parsed.
 *
 * Models draw diagrams in Mermaid because everything they were trained on
 * does, so a fence tagged `mermaid` is the one place a picture is genuinely
 * cheaper to produce than to describe. The official renderer is a megabyte of
 * dependency and this app takes none, so this is a reader for the part of the
 * language that actually turns up in a reply: flowcharts and sequence
 * diagrams.
 *
 * The contract with the rest of the app is the important bit. `parseMermaid`
 * returns a diagram or `null`, and `null` means "render the fence as code, the
 * way it rendered before this file existed". A class diagram, a Gantt chart or
 * a syntax error all take that path. Nothing here throws, and nothing here
 * guesses: a diagram that came back is one every line of which was understood.
 */

/* -- shared --------------------------------------------------------------- */

/** `<br/>`, the one piece of markup Mermaid labels really do carry. */
function label(raw) {
  return String(raw ?? "")
    .trim()
    .replace(/^["'`]|["'`]$/g, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/#quot;/g, '"')
    .replace(/\\n/g, "\n")
    .trim();
}

/** Statements, with comments, directives and blank lines gone. */
function statements(body) {
  const out = [];
  for (const rawLine of body.split("\n")) {
    const withoutComment = rawLine.replace(/%%.*$/, "");
    for (const part of withoutComment.split(";")) {
      const line = part.trim();
      if (line) out.push(line);
    }
  }
  return out;
}

/* -- flowcharts ----------------------------------------------------------- */

/**
 * Node shapes, longest delimiter first.
 *
 * The order is the whole trick: `[[a]]` has to be tried before `[a]`, or a
 * subroutine box parses as a rectangle whose label starts with a bracket.
 */
const SHAPES = [
  ["([", "])", "stadium"],
  ["[[", "]]", "subroutine"],
  ["[(", ")]", "cylinder"],
  ["((", "))", "circle"],
  ["{{", "}}", "hexagon"],
  ["[/", "/]", "parallelogram"],
  ["[\\", "\\]", "parallelogram"],
  ["[", "]", "rect"],
  ["(", ")", "round"],
  ["{", "}", "diamond"],
  [">", "]", "flag"],
];

const DIRECTIONS = new Set(["TD", "TB", "BT", "LR", "RL"]);

/** Ignored, but not a failure: styling says nothing about the shape. */
const FLOW_NOISE = /^(classDef|class|style|linkStyle|click|direction|accTitle|accDescr)\b/i;

/**
 * The link forms, longest first. `-.->` must beat `-.-`, `-->` must beat `--`.
 * `stroke` is what it looks like; `head` is whether it ends in an arrow.
 */
const LINKS = [
  ["<-->", "solid", "both"],
  ["<==>", "thick", "both"],
  ["-.->", "dotted", "end"],
  ["<-.->", "dotted", "both"],
  ["==>", "thick", "end"],
  ["===", "thick", "none"],
  ["-->", "solid", "end"],
  ["--x", "solid", "cross"],
  ["--o", "solid", "circle"],
  ["---", "solid", "none"],
  ["-.-", "dotted", "none"],
  ["==", "thick", "none"],
  ["--", "solid", "none"],
  ["->", "solid", "end"],
];

/**
 * `A -- yes --> B` written as `A -->|yes| B`.
 *
 * Mermaid has two spellings for an edge label and the rest of this file only
 * wants to know one, so the middle-text form is rewritten into the pipe form
 * before anything else looks at the line.
 */
function normaliseEdgeLabels(line) {
  return line.replace(
    /(--|==|-\.)[ \t]+([^|\n]+?)[ \t]+(-{2,3}[>xo]?|={2,3}>?|\.-{1,2}>?)/g,
    (whole, left, text, right) => {
      const head = right.includes(">") ? ">" : right.endsWith("x") ? "x" : right.endsWith("o") ? "o" : "";
      const stroke = left === "==" ? "==" : left === "-." ? "-." : "--";
      const arrow =
        stroke === "-." ? (head ? "-.->" : "-.-") : stroke === "==" ? (head ? "==>" : "===") : head ? `--${head}` : "---";
      return `${arrow}|${text}|`;
    }
  );
}

/** The node id and label at the start of `text`, or null. */
function readNode(text) {
  const idMatch = /^([A-Za-z0-9_.\-À-￿]+)/.exec(text);
  if (!idMatch) return null;
  const id = idMatch[1];
  let rest = text.slice(id.length);

  for (const [open, close, shape] of SHAPES) {
    if (!rest.startsWith(open)) continue;
    const end = rest.indexOf(close, open.length);
    if (end === -1) continue;
    return {
      id,
      text: label(rest.slice(open.length, end)),
      shape,
      rest: rest.slice(end + close.length),
    };
  }
  return { id, text: null, shape: null, rest };
}

/** The link at the start of `text`, with its optional `|label|`, or null. */
function readLink(text) {
  const trimmed = text.trimStart();
  const eaten = text.length - trimmed.length;
  for (const [token, stroke, head] of LINKS) {
    if (!trimmed.startsWith(token)) continue;
    let rest = trimmed.slice(token.length);
    let text_ = null;
    if (rest.startsWith("|")) {
      const end = rest.indexOf("|", 1);
      if (end !== -1) {
        text_ = label(rest.slice(1, end));
        rest = rest.slice(end + 1);
      }
    }
    return { stroke, head, label: text_, rest, consumed: eaten + token.length };
  }
  return null;
}

function parseFlowchart(header, body) {
  const direction = (/(TD|TB|BT|LR|RL)\s*$/i.exec(header.trim())?.[1] ?? "TD").toUpperCase();
  if (!DIRECTIONS.has(direction)) return null;

  const nodes = new Map();
  const edges = [];
  const groups = [];
  const stack = [];

  const note = (id, text, shape) => {
    const existing = nodes.get(id);
    if (!existing) {
      nodes.set(id, { id, label: text ?? id, shape: shape ?? "rect" });
    } else if (text !== null && text !== undefined) {
      existing.label = text;
      if (shape) existing.shape = shape;
    }
    if (stack.length) stack[stack.length - 1].nodes.add(id);
    return id;
  };

  for (const raw of statements(body)) {
    if (FLOW_NOISE.test(raw)) continue;

    const subgraph = /^subgraph\s+(.*)$/i.exec(raw);
    if (subgraph) {
      const declared = subgraph[1].trim();
      const titled = /^([A-Za-z0-9_.-]+)\s*\[(.*)\]$/.exec(declared);
      const group = {
        id: titled ? titled[1] : declared,
        label: label(titled ? titled[2] : declared),
        nodes: new Set(),
      };
      groups.push(group);
      stack.push(group);
      continue;
    }
    if (/^end$/i.test(raw)) {
      stack.pop();
      continue;
    }

    const line = normaliseEdgeLabels(raw);
    let cursor = 0;
    let previous = null;
    let sawLink = false;
    let ok = true;

    while (cursor < line.length) {
      const remainder = line.slice(cursor).trimStart();
      if (!remainder) break;
      cursor = line.length - remainder.length;

      const node = readNode(line.slice(cursor));
      if (!node) {
        ok = false;
        break;
      }
      const id = note(node.id, node.text, node.shape);
      cursor = line.length - node.rest.length;

      if (previous && sawLink) {
        edges.push({ ...sawLink, from: previous, to: id });
        sawLink = false;
      }
      previous = id;

      const link = readLink(line.slice(cursor));
      if (!link) break;
      sawLink = { stroke: link.stroke, head: link.head, label: link.label };
      cursor = line.length - link.rest.length;
    }

    // A statement that is one bare word is a node on its own, which is legal.
    // A statement this could not finish reading means the diagram has syntax
    // outside what is understood here, and a half-read diagram is worse than
    // a code block, so the whole thing goes back as unparsed.
    if (!ok || sawLink) return null;
  }

  if (!nodes.size) return null;
  return {
    kind: "flowchart",
    direction,
    nodes: [...nodes.values()],
    edges,
    groups: groups
      .filter((group) => group.nodes.size)
      .map((group) => ({ id: group.id, label: group.label, nodes: [...group.nodes] })),
  };
}

/* -- sequence diagrams ---------------------------------------------------- */

const SEQUENCE_ARROWS = [
  ["-->>", "dashed", true],
  ["->>", "solid", true],
  ["--x", "dashed", true],
  ["-x", "solid", true],
  ["--)", "dashed", true],
  ["-)", "solid", true],
  ["-->", "dashed", false],
  ["->", "solid", false],
];

const SEQUENCE_NOISE =
  /^(autonumber|activate|deactivate|box|end|rect|critical|break|par|and|options|accTitle|accDescr|link|links)\b/i;

const SEQUENCE_BLOCK = /^(loop|alt|else|opt|par)\b\s*(.*)$/i;

function parseSequence(body) {
  const actors = [];
  const index = new Map();
  const steps = [];

  const actor = (raw) => {
    const id = raw.trim();
    if (!id) return null;
    if (!index.has(id)) {
      index.set(id, actors.length);
      actors.push({ id, label: label(id) });
    }
    return id;
  };

  for (const raw of statements(body)) {
    const declared = /^(participant|actor)\s+(.+)$/i.exec(raw);
    if (declared) {
      const parts = /^(.+?)\s+as\s+(.+)$/i.exec(declared[2].trim());
      const id = (parts ? parts[1] : declared[2]).trim();
      actor(id);
      if (parts) actors[index.get(id)].label = label(parts[2]);
      continue;
    }

    const note = /^note\s+(over|left of|right of)\s+([^:]+):\s*(.*)$/i.exec(raw);
    if (note) {
      const who = note[2].split(",").map((name) => actor(name));
      const where = note[1].toLowerCase();
      steps.push({
        type: "note",
        // Which side matters: a note "right of" the server drawn on top of its
        // lifeline hides the thing it is about.
        side: where === "over" ? "over" : where === "left of" ? "left" : "right",
        actors: who.filter(Boolean),
        label: label(note[3]),
      });
      continue;
    }

    const block = SEQUENCE_BLOCK.exec(raw);
    if (block) {
      steps.push({ type: "block", keyword: block[1].toLowerCase(), label: label(block[2]) });
      continue;
    }

    if (SEQUENCE_NOISE.test(raw)) continue;

    const arrow = SEQUENCE_ARROWS.find((entry) => raw.includes(entry[0]));
    if (!arrow) return null;
    const [token, stroke, head] = arrow;
    const at = raw.indexOf(token);
    const from = actor(raw.slice(0, at));
    const after = raw.slice(at + token.length);
    const colon = after.indexOf(":");
    const to = actor(colon === -1 ? after : after.slice(0, colon));
    if (!from || !to) return null;
    steps.push({
      type: "message",
      from,
      to,
      stroke,
      head,
      label: colon === -1 ? "" : label(after.slice(colon + 1)),
    });
  }

  if (actors.length < 1 || !steps.some((step) => step.type === "message")) return null;
  return { kind: "sequence", actors, steps };
}

/* -- the door ------------------------------------------------------------- */

/**
 * A diagram, or `null` when this cannot read it.
 *
 * `null` is not a failure state to be handled somewhere - it is the ordinary
 * answer for every diagram type this does not draw, and the caller's response
 * to it is to render the fence as code.
 */
export function parseMermaid(source) {
  const text = String(source ?? "").replace(/\r\n?/g, "\n");
  const lines = text.split("\n");
  let start = 0;
  // An `%%{init: ...}%%` directive and any blank lines come before the header.
  while (start < lines.length && (!lines[start].trim() || lines[start].trim().startsWith("%%"))) {
    start += 1;
  }
  const header = lines[start] ?? "";
  const body = lines.slice(start + 1).join("\n");

  try {
    if (/^\s*(flowchart|graph)\b/i.test(header)) return parseFlowchart(header, body);
    if (/^\s*sequenceDiagram\b/i.test(header)) return parseSequence(body);
    return null;
  } catch {
    return null;
  }
}
