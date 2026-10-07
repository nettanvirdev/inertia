/**
 * TeX, parsed - the part of it people actually write in a message.
 *
 * A model asked about gradient descent writes `\frac{\partial L}{\partial w}`,
 * and a chat that shows that as literal backslashes has failed at the one
 * thing it is for. The real answer is KaTeX; the real answer is also 300 KB of
 * dependency, and this app takes none. So this reads the subset that turns up
 * in an explanation - fractions, roots, sums and integrals with their limits,
 * sub- and superscripts, Greek, the relation and arrow symbols, upright
 * function names, accents, and small matrices - into a tree the renderer lays
 * out with CSS.
 *
 * What it does with the rest is the important part: an unknown command becomes
 * a text node holding the command as written. A formula using something not
 * implemented here comes out as mathematics with one odd word in it, rather
 * than as an exception or as a blank space. Nothing here throws.
 *
 * Nodes:
 *   { type: "text", value, upright? }
 *   { type: "row", body }
 *   { type: "frac", num, den }
 *   { type: "sqrt", body, index }
 *   { type: "script", base, sup, sub, limits }
 *   { type: "accent", accent, body }
 *   { type: "space", size }
 *   { type: "matrix", rows, open, close }
 */

const SYMBOLS = {
  alpha: "α",
  beta: "β",
  gamma: "γ",
  delta: "δ",
  epsilon: "ε",
  varepsilon: "ε",
  zeta: "ζ",
  eta: "η",
  theta: "θ",
  vartheta: "ϑ",
  iota: "ι",
  kappa: "κ",
  lambda: "λ",
  mu: "μ",
  nu: "ν",
  xi: "ξ",
  pi: "π",
  rho: "ρ",
  sigma: "σ",
  tau: "τ",
  upsilon: "υ",
  phi: "φ",
  varphi: "ϕ",
  chi: "χ",
  psi: "ψ",
  omega: "ω",
  Gamma: "Γ",
  Delta: "Δ",
  Theta: "Θ",
  Lambda: "Λ",
  Xi: "Ξ",
  Pi: "Π",
  Sigma: "Σ",
  Upsilon: "Υ",
  Phi: "Φ",
  Psi: "Ψ",
  Omega: "Ω",

  times: "×",
  div: "÷",
  pm: "±",
  mp: "∓",
  cdot: "⋅",
  ast: "∗",
  star: "⋆",
  circ: "∘",
  bullet: "•",
  oplus: "⊕",
  otimes: "⊗",
  leq: "≤",
  le: "≤",
  geq: "≥",
  ge: "≥",
  neq: "≠",
  ne: "≠",
  approx: "≈",
  equiv: "≡",
  sim: "∼",
  simeq: "≃",
  cong: "≅",
  propto: "∝",
  ll: "≪",
  gg: "≫",
  in: "∈",
  notin: "∉",
  ni: "∋",
  subset: "⊂",
  subseteq: "⊆",
  supset: "⊃",
  supseteq: "⊇",
  cup: "∪",
  cap: "∩",
  setminus: "∖",
  emptyset: "∅",
  varnothing: "∅",
  infty: "∞",
  partial: "∂",
  nabla: "∇",
  forall: "∀",
  exists: "∃",
  neg: "¬",
  lnot: "¬",
  land: "∧",
  wedge: "∧",
  lor: "∨",
  vee: "∨",
  to: "→",
  rightarrow: "→",
  longrightarrow: "⟶",
  leftarrow: "←",
  longleftarrow: "⟵",
  leftrightarrow: "↔",
  Rightarrow: "⇒",
  Leftarrow: "⇐",
  Leftrightarrow: "⇔",
  implies: "⟹",
  iff: "⟺",
  mapsto: "↦",
  uparrow: "↑",
  downarrow: "↓",
  dots: "…",
  ldots: "…",
  cdots: "⋯",
  vdots: "⋮",
  ddots: "⋱",
  prime: "′",
  degree: "°",
  hbar: "ℏ",
  ell: "ℓ",
  Re: "ℜ",
  Im: "ℑ",
  aleph: "ℵ",
  perp: "⊥",
  parallel: "∥",
  angle: "∠",
  triangle: "△",
  square: "□",
  checkmark: "✓",
  dagger: "†",
  surd: "√",
  mid: "|",
  backslash: "\\",
  lbrace: "{",
  rbrace: "}",
  langle: "⟨",
  rangle: "⟩",
  lceil: "⌈",
  rceil: "⌉",
  lfloor: "⌊",
  rfloor: "⌋",
  quad: " ",
  qquad: "  ",
};

/** Operators that carry their limits above and below in a display formula. */
const BIG_OPERATORS = {
  sum: "∑",
  prod: "∏",
  coprod: "∐",
  int: "∫",
  iint: "∬",
  iiint: "∭",
  oint: "∮",
  bigcup: "⋃",
  bigcap: "⋂",
  bigoplus: "⨁",
  bigotimes: "⨂",
  bigvee: "⋁",
  bigwedge: "⋀",
  lim: "lim",
  limsup: "lim sup",
  liminf: "lim inf",
  max: "max",
  min: "min",
  sup: "sup",
  inf: "inf",
  argmax: "arg max",
  argmin: "arg min",
};

/** Names set upright, because `sin` in italics reads as s times i times n. */
const FUNCTIONS = new Set([
  "sin",
  "cos",
  "tan",
  "cot",
  "sec",
  "csc",
  "arcsin",
  "arccos",
  "arctan",
  "sinh",
  "cosh",
  "tanh",
  "log",
  "ln",
  "lg",
  "exp",
  "det",
  "dim",
  "ker",
  "deg",
  "gcd",
  "hom",
  "arg",
  "Pr",
  "mod",
  "bmod",
]);

const ACCENTS = {
  hat: "̂",
  widehat: "̂",
  bar: "̄",
  overline: "̄",
  vec: "⃗",
  tilde: "̃",
  widetilde: "̃",
  dot: "̇",
  ddot: "̈",
  check: "̌",
  breve: "̆",
  acute: "́",
  grave: "̀",
};

const STYLES = {
  text: "upright",
  textrm: "upright",
  mathrm: "upright",
  operatorname: "upright",
  mathbf: "bold",
  textbf: "bold",
  bf: "bold",
  mathbb: "bold",
  mathcal: "italic",
  mathit: "italic",
  textit: "italic",
  mathsf: "upright",
  mathtt: "mono",
  texttt: "mono",
};

const SPACES = { ",": 0.17, ":": 0.22, ";": 0.28, "!": -0.17, " ": 0.25 };

const MATRIX_DELIMS = {
  matrix: ["", ""],
  pmatrix: ["(", ")"],
  bmatrix: ["[", "]"],
  Bmatrix: ["{", "}"],
  vmatrix: ["|", "|"],
  Vmatrix: ["‖", "‖"],
  cases: ["{", ""],
  aligned: ["", ""],
  array: ["", ""],
};

class Reader {
  constructor(source) {
    this.src = source;
    this.at = 0;
  }

  get done() {
    return this.at >= this.src.length;
  }

  peek() {
    return this.src[this.at];
  }

  /** The next control sequence, `{`, `}` or single character. */
  next() {
    const ch = this.src[this.at];
    if (ch !== "\\") {
      this.at += 1;
      return ch;
    }
    const rest = this.src.slice(this.at + 1);
    const word = /^[A-Za-z]+/.exec(rest);
    if (word) {
      this.at += 1 + word[0].length;
      return `\\${word[0]}`;
    }
    this.at += 2;
    return `\\${rest[0] ?? ""}`;
  }
}

function textNode(value, upright = false) {
  return { type: "text", value, upright };
}

/** A `{...}` group, or the single atom that follows a command. */
function readArgument(reader) {
  while (reader.peek() === " ") reader.at += 1;
  if (reader.done) return { type: "row", body: [] };
  if (reader.peek() === "{") {
    reader.at += 1;
    return readRow(reader, "}");
  }
  const token = reader.next();
  return atomFor(token, reader);
}

function readRow(reader, stop) {
  const body = [];
  while (!reader.done) {
    if (stop === "}" && reader.peek() === "}") {
      reader.at += 1;
      break;
    }
    const before = reader.at;
    const token = reader.next();
    if (token === "&" || token === "\\\\") {
      // The caller (a matrix) wants these; anything else drops them.
      reader.at = before;
      break;
    }
    if (token === "^" || token === "_") {
      const previous = body.pop() ?? { type: "row", body: [] };
      const script =
        previous.type === "script" && !previous[token === "^" ? "sup" : "sub"]
          ? previous
          : { type: "script", base: previous, sup: null, sub: null, limits: false };
      script[token === "^" ? "sup" : "sub"] = readArgument(reader);
      if (previous.type === "script" && script !== previous) script.limits = previous.limits;
      if (previous.limits) script.limits = true;
      body.push(script);
      continue;
    }
    const atom = atomFor(token, reader);
    if (atom) body.push(atom);
  }
  return { type: "row", body };
}

function readMatrix(reader, name) {
  const rows = [[]];
  let cell = [];
  const flushCell = () => {
    rows[rows.length - 1].push({ type: "row", body: cell });
    cell = [];
  };

  while (!reader.done) {
    if (reader.src.startsWith(`\\end{${name}}`, reader.at)) {
      reader.at += `\\end{${name}}`.length;
      break;
    }
    const before = reader.at;
    const token = reader.next();
    if (token === "&") {
      flushCell();
      continue;
    }
    if (token === "\\\\") {
      flushCell();
      rows.push([]);
      continue;
    }
    if (token === "^" || token === "_") {
      const previous = cell.pop() ?? { type: "row", body: [] };
      const script = { type: "script", base: previous, sup: null, sub: null, limits: false };
      script[token === "^" ? "sup" : "sub"] = readArgument(reader);
      cell.push(script);
      continue;
    }
    if (reader.at === before) break; // nothing consumed: refuse to spin
    const atom = atomFor(token, reader);
    if (atom) cell.push(atom);
  }
  flushCell();
  const [open, close] = MATRIX_DELIMS[name] ?? ["", ""];
  return {
    type: "matrix",
    rows: rows.filter((row) => row.some((entry) => entry.body.length)),
    open,
    close,
  };
}

function atomFor(token, reader) {
  if (token === undefined) return null;
  if (token === "{") return readRow(reader, "}");
  if (token === "}") return null;
  if (token === " ") return { type: "space", size: 0.22 };
  if (token === "~") return { type: "space", size: 0.25 };

  if (!token.startsWith("\\")) {
    // Digits run together so `1024` is one node rather than four.
    if (/[0-9.]/.test(token)) {
      let value = token;
      while (!reader.done && /[0-9.]/.test(reader.peek())) value += reader.next();
      return textNode(value, true);
    }
    if (/[A-Za-z]/.test(token)) return textNode(token, false);
    return textNode(token, true);
  }

  const name = token.slice(1);

  if (name === "frac" || name === "dfrac" || name === "tfrac") {
    return { type: "frac", num: readArgument(reader), den: readArgument(reader) };
  }
  if (name === "sqrt") {
    let index = null;
    if (reader.peek() === "[") {
      const end = reader.src.indexOf("]", reader.at);
      if (end !== -1) {
        index = parse(reader.src.slice(reader.at + 1, end));
        reader.at = end + 1;
      }
    }
    return { type: "sqrt", index, body: readArgument(reader) };
  }
  if (name === "begin") {
    const env = /^\{([A-Za-z*]+)\}/.exec(reader.src.slice(reader.at));
    if (env) {
      reader.at += env[0].length;
      // `array` takes a column spec nobody reading a message needs to see.
      if (env[1] === "array" && reader.peek() === "{") {
        const close = reader.src.indexOf("}", reader.at);
        if (close !== -1) reader.at = close + 1;
      }
      return readMatrix(reader, env[1]);
    }
    return textNode("\\begin", true);
  }
  if (name === "end") {
    const env = /^\{[A-Za-z*]+\}/.exec(reader.src.slice(reader.at));
    if (env) reader.at += env[0].length;
    return null;
  }
  if (name === "left" || name === "right") {
    const delim = reader.done ? "" : reader.next();
    const value = delim.startsWith("\\")
      ? (SYMBOLS[delim.slice(1)] ?? "")
      : delim === "."
        ? ""
        : delim;
    return value ? textNode(value, true) : null;
  }
  if (ACCENTS[name]) return { type: "accent", accent: ACCENTS[name], body: readArgument(reader) };
  if (STYLES[name]) {
    const body = readArgument(reader);
    return { type: "styled", style: STYLES[name], body };
  }
  if (BIG_OPERATORS[name]) {
    return {
      type: "script",
      base: textNode(BIG_OPERATORS[name], true),
      sup: null,
      sub: null,
      limits: true,
    };
  }
  if (FUNCTIONS.has(name)) return textNode(name, true);
  if (SPACES[name] !== undefined) return { type: "space", size: SPACES[name] };
  if (SYMBOLS[name] !== undefined) return textNode(SYMBOLS[name], true);
  if (name === "\\") return { type: "break" };
  if (name === "&" || name === "%" || name === "$" || name === "#" || name === "_") {
    return textNode(name, true);
  }
  // Unknown: show it as written. A formula with one strange word in it is
  // still a formula; a thrown error is a message that fails to render.
  return textNode(token, true);
}

/**
 * A formula, as a tree. Never throws: anything that cannot be read comes back
 * as the text it was written as.
 */
export function parseMath(source) {
  try {
    return readRow(new Reader(String(source ?? "").trim()), null);
  } catch {
    return { type: "row", body: [textNode(String(source ?? ""), true)] };
  }
}

/** Alias used inside this file for nested parses. */
const parse = parseMath;

export { SYMBOLS, BIG_OPERATORS, FUNCTIONS };
