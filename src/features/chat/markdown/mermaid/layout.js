/**
 * Where the boxes go.
 *
 * A diagram is a graph with no coordinates, and turning one into the other is
 * the whole job of a diagramming library. This is the small version of the
 * layered method every one of them uses - rank, order, place, route - written
 * as pure functions over the shapes `parse.js` produces, so the geometry can
 * be tested without a DOM and the renderer stays a mapping from numbers to
 * SVG.
 *
 * Text is measured by counting characters, because measuring it properly means
 * having a page. The constants below are the width of a character in the app's
 * own type at the size the diagram is drawn, rounded up: a box slightly too
 * wide is invisible, and a box slightly too narrow has its label hanging out
 * of it, so every estimate here rounds the safe way.
 */

const CHAR = 6.9;
const LINE = 17;
const PAD_X = 14;
const PAD_Y = 11;
const MIN_W = 46;
const MIN_H = 34;

/** Rank to rank, along the direction of flow. */
const RANK_GAP = 56;
/** Between two boxes in the same rank. */
const NODE_GAP = 26;
const MARGIN = 12;

function measure(text) {
  const lines = String(text ?? "").split("\n");
  const width = Math.max(...lines.map((line) => line.length), 1) * CHAR;
  return { lines, width, height: lines.length * LINE };
}

function sizeOf(node) {
  const { lines, width, height } = measure(node.label);
  let w = Math.max(width + PAD_X * 2, MIN_W);
  let h = Math.max(height + PAD_Y * 2, MIN_H);
  // A diamond only touches its label at the middle of each edge, and a circle
  // is worse; both need room the rectangle does not.
  if (node.shape === "diamond") {
    w += width * 0.5 + 12;
    h += 14;
  }
  if (node.shape === "circle") {
    const side = Math.max(w, h) + 10;
    w = side;
    h = side;
  }
  if (node.shape === "hexagon" || node.shape === "parallelogram") w += 16;
  if (node.shape === "cylinder") h += 10;
  return { w: Math.round(w), h: Math.round(h), lines };
}

/* -- flowchart ------------------------------------------------------------ */

/**
 * Ranks, by longest path.
 *
 * Edges that close a cycle are left out of the ranking and drawn afterwards -
 * a diagram with a loop in it is common and perfectly readable, but a longest
 * path through a cycle does not terminate.
 */
function rankNodes(ids, edges) {
  const out = new Map(ids.map((id) => [id, []]));
  for (const edge of edges) {
    if (edge.from !== edge.to) out.get(edge.from)?.push(edge.to);
  }

  const state = new Map(ids.map((id) => [id, 0]));
  const back = new Set();
  const stack = [];
  const walk = (id) => {
    state.set(id, 1);
    stack.push(id);
    for (const next of out.get(id) ?? []) {
      if (state.get(next) === 1) back.add(`${id} ${next}`);
      else if (state.get(next) === 0) walk(next);
    }
    state.set(id, 2);
    stack.pop();
  };
  for (const id of ids) if (state.get(id) === 0) walk(id);

  const forward = edges.filter(
    (edge) => edge.from !== edge.to && !back.has(`${edge.from} ${edge.to}`)
  );
  const rank = new Map(ids.map((id) => [id, 0]));
  // Relax until nothing moves. The graph is a DAG once the back edges are out,
  // so this settles in at most `ids.length` rounds.
  for (let round = 0; round < ids.length; round += 1) {
    let moved = false;
    for (const edge of forward) {
      const wanted = rank.get(edge.from) + 1;
      if (wanted > rank.get(edge.to)) {
        rank.set(edge.to, wanted);
        moved = true;
      }
    }
    if (!moved) break;
  }
  return rank;
}

/** The median of a node's neighbours in the rank above, for crossing removal. */
function median(values) {
  if (!values.length) return -1;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function orderRanks(layers, edges) {
  const neighbours = new Map();
  const add = (key, value) => {
    if (!neighbours.has(key)) neighbours.set(key, []);
    neighbours.get(key).push(value);
  };
  for (const edge of edges) {
    if (edge.from === edge.to) continue;
    add(`d:${edge.to}`, edge.from);
    add(`u:${edge.from}`, edge.to);
  }

  const position = new Map();
  const reindex = () => {
    layers.forEach((layer) => layer.forEach((id, at) => position.set(id, at)));
  };
  reindex();

  // Four sweeps is where this stops being worth it: the median heuristic gets
  // most of its improvement in the first two, and a chat message is not the
  // place to spend milliseconds chasing the last crossing.
  for (let pass = 0; pass < 4; pass += 1) {
    const downward = pass % 2 === 0;
    const order = downward ? [...layers.keys()] : [...layers.keys()].reverse();
    for (const index of order) {
      const side = downward ? "d" : "u";
      const layer = layers[index];
      const keyed = layer.map((id, at) => ({
        id,
        at,
        key: median(
          (neighbours.get(`${side}:${id}`) ?? []).map((other) => position.get(other) ?? -1)
        ),
      }));
      keyed.sort((a, b) => {
        if (a.key === -1 || b.key === -1) return a.at - b.at;
        return a.key === b.key ? a.at - b.at : a.key - b.key;
      });
      layers[index] = keyed.map((entry) => entry.id);
      reindex();
    }
  }
  return layers;
}

/** Where a straight line from the centre of a box leaves it. */
function edgeOfBox(node, towardsX, towardsY) {
  const dx = towardsX - node.cx;
  const dy = towardsY - node.cy;
  if (!dx && !dy) return { x: node.cx, y: node.cy };
  const halfW = node.w / 2;
  const halfH = node.h / 2;
  const scale = Math.min(
    dx ? halfW / Math.abs(dx) : Infinity,
    dy ? halfH / Math.abs(dy) : Infinity
  );
  return { x: node.cx + dx * scale, y: node.cy + dy * scale };
}

export function layoutFlowchart(diagram) {
  const vertical =
    diagram.direction === "TD" || diagram.direction === "TB" || diagram.direction === "BT";
  const reversed = diagram.direction === "BT" || diagram.direction === "RL";

  const sized = new Map(diagram.nodes.map((node) => [node.id, { ...node, ...sizeOf(node) }]));
  const ids = diagram.nodes.map((node) => node.id);
  const rank = rankNodes(ids, diagram.edges);

  const depth = Math.max(...ids.map((id) => rank.get(id))) + 1;
  const layers = Array.from({ length: depth }, () => []);
  for (const id of ids) layers[rank.get(id)].push(id);
  orderRanks(layers, diagram.edges);

  // Along the flow: each rank sits after the deepest box of the one before it.
  const rankOffset = [];
  let along = MARGIN;
  layers.forEach((layer, index) => {
    rankOffset[index] = along;
    const thickness = Math.max(
      ...layer.map((id) => (vertical ? sized.get(id).h : sized.get(id).w)),
      MIN_H
    );
    along += thickness + RANK_GAP;
  });
  const alongTotal = along - RANK_GAP + MARGIN;

  // Across the flow: lay each rank out in order, then centre every rank on the
  // widest one, so a three-box row is not left-aligned under a six-box row.
  const across = layers.map((layer) => {
    let at = 0;
    const places = layer.map((id) => {
      const node = sized.get(id);
      const span = vertical ? node.w : node.h;
      const place = { id, start: at, span };
      at += span + NODE_GAP;
      return place;
    });
    return { places, total: Math.max(at - NODE_GAP, 0) };
  });
  const acrossTotal = Math.max(...across.map((row) => row.total), MIN_W) + MARGIN * 2;

  const placed = new Map();
  layers.forEach((layer, index) => {
    const row = across[index];
    const shift = (acrossTotal - row.total) / 2;
    const alongStart = rankOffset[index];
    const thickness = Math.max(
      ...layer.map((id) => (vertical ? sized.get(id).h : sized.get(id).w)),
      MIN_H
    );
    for (const place of row.places) {
      const node = sized.get(place.id);
      const crossCentre = shift + place.start + place.span / 2;
      const alongCentre =
        (reversed ? alongTotal - MARGIN - (alongStart - MARGIN) - thickness : alongStart) +
        thickness / 2;
      placed.set(place.id, {
        ...node,
        cx: vertical ? crossCentre : alongCentre,
        cy: vertical ? alongCentre : crossCentre,
      });
    }
  });

  const width = vertical ? acrossTotal : alongTotal;
  const height = vertical ? alongTotal : acrossTotal;

  const nodes = [...placed.values()].map((node) => ({
    id: node.id,
    label: node.label,
    lines: node.lines,
    shape: node.shape,
    x: Math.round(node.cx - node.w / 2),
    y: Math.round(node.cy - node.h / 2),
    w: node.w,
    h: node.h,
    cx: Math.round(node.cx),
    cy: Math.round(node.cy),
  }));

  const edges = diagram.edges.map((edge) => {
    const from = placed.get(edge.from);
    const to = placed.get(edge.to);
    if (!from || !to) return null;

    if (from === to) {
      // A self-loop leaves and re-enters the same side, drawn as a small ear
      // so it cannot be mistaken for an edge to the neighbour.
      const r = 18;
      const path = `M ${from.cx + from.w / 2} ${from.cy - 6} C ${from.cx + from.w / 2 + r * 2} ${
        from.cy - r
      }, ${from.cx + from.w / 2 + r * 2} ${from.cy + r}, ${from.cx + from.w / 2} ${from.cy + 6}`;
      return {
        ...edge,
        path,
        labelX: from.cx + from.w / 2 + r * 1.6,
        labelY: from.cy,
        endX: from.cx + from.w / 2,
        endY: from.cy + 6,
        angle: 160,
      };
    }

    const start = edgeOfBox(from, to.cx, to.cy);
    const end = edgeOfBox(to, from.cx, from.cy);
    // One control point each, pulled along the flow axis: enough to keep two
    // edges into the same box distinguishable, not enough to make a short
    // link look like a river.
    const bend = Math.min(Math.abs(vertical ? end.y - start.y : end.x - start.x) / 2, 34);
    const c1 = vertical
      ? `${start.x} ${start.y + Math.sign(end.y - start.y) * bend}`
      : `${start.x + Math.sign(end.x - start.x) * bend} ${start.y}`;
    const c2 = vertical
      ? `${end.x} ${end.y - Math.sign(end.y - start.y) * bend}`
      : `${end.x - Math.sign(end.x - start.x) * bend} ${end.y}`;

    return {
      ...edge,
      path: `M ${start.x} ${start.y} C ${c1}, ${c2}, ${end.x} ${end.y}`,
      labelX: (start.x + end.x) / 2,
      labelY: (start.y + end.y) / 2,
      endX: end.x,
      endY: end.y,
      angle: (Math.atan2(end.y - start.y, end.x - start.x) * 180) / Math.PI,
    };
  });

  const groups = diagram.groups
    .map((group) => {
      const members = group.nodes.map((id) => placed.get(id)).filter(Boolean);
      if (!members.length) return null;
      const pad = 14;
      const top = Math.min(...members.map((node) => node.cy - node.h / 2)) - pad - 16;
      const left = Math.min(...members.map((node) => node.cx - node.w / 2)) - pad;
      const right = Math.max(...members.map((node) => node.cx + node.w / 2)) + pad;
      const bottom = Math.max(...members.map((node) => node.cy + node.h / 2)) + pad;
      return {
        id: group.id,
        label: group.label,
        x: Math.round(left),
        y: Math.round(top),
        w: Math.round(right - left),
        h: Math.round(bottom - top),
      };
    })
    .filter(Boolean);

  // A group box can reach past the nodes it holds, and a label above the top
  // rank can reach above the canvas; both are fixed by growing the frame.
  const minX = Math.min(0, ...groups.map((group) => group.x - 4));
  const minY = Math.min(0, ...groups.map((group) => group.y - 4));
  const maxX = Math.max(width, ...groups.map((group) => group.x + group.w + 4));
  const maxY = Math.max(height, ...groups.map((group) => group.y + group.h + 4));

  return {
    kind: "flowchart",
    viewBox: { x: minX, y: minY, w: maxX - minX, h: maxY - minY },
    width: Math.round(maxX - minX),
    height: Math.round(maxY - minY),
    nodes,
    edges: edges.filter(Boolean),
    groups,
  };
}

/* -- sequence ------------------------------------------------------------- */

const ACTOR_GAP = 40;
const STEP_GAP = 44;
const NOTE_PAD = 10;

export function layoutSequence(diagram) {
  const actors = diagram.actors.map((actor) => {
    const { width } = measure(actor.label);
    return { ...actor, w: Math.max(Math.round(width + PAD_X * 2), 72), h: 34 };
  });

  // Two actors must be far enough apart that the longest message between them
  // still fits over the line, or every label overlaps its neighbour.
  const widest = new Map();
  for (const step of diagram.steps) {
    if (step.type !== "message" || step.from === step.to) continue;
    const key = [step.from, step.to].sort().join(" ");
    widest.set(key, Math.max(widest.get(key) ?? 0, measure(step.label).width + 24));
  }

  let x = MARGIN;
  const centre = new Map();
  actors.forEach((actor, index) => {
    if (index) {
      const previous = actors[index - 1];
      const key = [previous.id, actor.id].sort().join(" ");
      const needed = Math.max(
        previous.w / 2 + actor.w / 2 + ACTOR_GAP,
        (widest.get(key) ?? 0) + 20
      );
      x += needed;
    } else {
      x += actor.w / 2;
    }
    centre.set(actor.id, Math.round(x));
  });

  const top = MARGIN + 34;
  let y = top + 26;
  const steps = [];

  for (const step of diagram.steps) {
    if (step.type === "note") {
      const { width, lines } = measure(step.label);
      const xs = step.actors.map((id) => centre.get(id)).filter((value) => value !== undefined);
      const middle = xs.length ? (Math.min(...xs) + Math.max(...xs)) / 2 : MARGIN;
      // A note over two actors has to reach both lifelines, however short its
      // text is - a narrow box floating between them is not "over" anything.
      const span = xs.length > 1 ? Math.max(...xs) - Math.min(...xs) + 48 : 0;
      const w = Math.max(width + NOTE_PAD * 2, span, 80);
      const h = lines.length * LINE + NOTE_PAD * 2;
      const left =
        step.side === "right"
          ? Math.max(...xs) + 12
          : step.side === "left"
            ? Math.min(...xs) - 12 - w
            : middle - w / 2;
      steps.push({ ...step, x: Math.round(left), y, w: Math.round(w), h, lines });
      y += h + 16;
      continue;
    }
    if (step.type === "block") {
      steps.push({ ...step, y: y + 4 });
      y += 26;
      continue;
    }

    const fromX = centre.get(step.from);
    const toX = centre.get(step.to);
    if (fromX === undefined || toX === undefined) continue;
    if (step.from === step.to) {
      steps.push({ ...step, fromX, toX, y, selfHeight: 30, lines: measure(step.label).lines });
      y += 30 + STEP_GAP - 14;
      continue;
    }
    steps.push({ ...step, fromX, toX, y, lines: measure(step.label).lines });
    y += STEP_GAP;
  }

  const width = Math.round(
    Math.max(...actors.map((actor) => centre.get(actor.id) + actor.w / 2), 120) + MARGIN
  );
  const bottom = y + 10;

  return {
    kind: "sequence",
    viewBox: { x: 0, y: 0, w: width, h: Math.round(bottom + 34) },
    width,
    height: Math.round(bottom + 34),
    actors: actors.map((actor) => ({
      ...actor,
      cx: centre.get(actor.id),
      y: MARGIN,
      bottom,
      // The name is repeated at the foot of a long diagram, the way Mermaid
      // does it, because by the time you have scrolled to the last message you
      // can no longer see who the third lifeline was.
      footY: bottom,
    })),
    steps,
    lifelineTop: top,
    lifelineBottom: bottom,
  };
}

/** Geometry for a parsed diagram, or null if it is a kind with no layout. */
export function layoutDiagram(diagram) {
  if (!diagram) return null;
  if (diagram.kind === "flowchart") return layoutFlowchart(diagram);
  if (diagram.kind === "sequence") return layoutSequence(diagram);
  return null;
}
