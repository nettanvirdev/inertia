import { describe, expect, it } from "vitest";
import { parseMath } from "./parse.js";
import { parseInline } from "../parse-inline.js";
import { parseBlocks } from "../parse-blocks.js";

/** The text of a tree, flattened, so a test can say what it reads as. */
function flatten(node) {
  if (!node) return "";
  switch (node.type) {
    case "row":
      return node.body.map(flatten).join("");
    case "text":
      return node.value;
    case "styled":
      return flatten(node.body);
    case "frac":
      return `(${flatten(node.num)})/(${flatten(node.den)})`;
    case "sqrt":
      return `sqrt(${flatten(node.body)})`;
    case "script":
      return `${flatten(node.base)}${node.sub ? `_${flatten(node.sub)}` : ""}${node.sup ? `^${flatten(node.sup)}` : ""}`;
    case "accent":
      return `${flatten(node.body)}${node.accent}`;
    case "matrix":
      return node.rows.map((row) => row.map((cell) => flatten(cell).trim()).join(",")).join(";");
    case "space":
      return " ";
    default:
      return "";
  }
}

describe("reading a formula", () => {
  it("reads a fraction, a root and a script", () => {
    expect(flatten(parseMath("\\frac{a}{b}"))).toBe("(a)/(b)");
    // Spaces are kept as they were typed: TeX would set them from the kinds
    // of atom on either side, and reproducing that table is a lot of machinery
    // to arrive back at what the author already wrote.
    expect(flatten(parseMath("\\sqrt{x + 1}"))).toBe("sqrt(x + 1)");
    expect(flatten(parseMath("x^2_i"))).toBe("x_i^2");
  });

  it("keeps a multi-digit number together", () => {
    const tree = parseMath("1024 + 3.5");
    expect(tree.body[0]).toEqual({ type: "text", value: "1024", upright: true });
  });

  it("turns a named symbol into the character it stands for", () => {
    expect(flatten(parseMath("\\alpha + \\beta \\leq \\Omega"))).toBe("α + β ≤ Ω");
    expect(flatten(parseMath("x \\to \\infty"))).toBe("x → ∞");
  });

  it("gives a big operator its limits, and marks it as taking them", () => {
    const [sum] = parseMath("\\sum_{i=1}^{n} i").body;
    expect(sum.type).toBe("script");
    expect(sum.limits).toBe(true);
    expect(flatten(sum.sub)).toBe("i=1");
    expect(flatten(sum.sup)).toBe("n");
  });

  it("sets a function name upright, and a variable in italics", () => {
    const [fn, , variable] = parseMath("\\sin x").body;
    expect(fn).toEqual({ type: "text", value: "sin", upright: true });
    expect(variable.upright).toBe(false);
  });

  it("reads an accent as a mark over its argument", () => {
    const [hat] = parseMath("\\hat{y}").body;
    expect(hat.type).toBe("accent");
    expect(flatten(hat.body)).toBe("y");
  });

  it("drops the sizing of a delimiter but keeps the delimiter", () => {
    expect(flatten(parseMath("\\left( x \\right)"))).toBe("( x )");
    expect(flatten(parseMath("\\left| x \\right|"))).toBe("| x |");
  });

  it("reads a matrix into rows and columns", () => {
    const matrix = parseMath("\\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}").body[0];
    expect(matrix.type).toBe("matrix");
    expect(matrix.open).toBe("(");
    expect(flatten(matrix)).toBe("a,b;c,d");
  });

  it("shows a command it does not know rather than failing", () => {
    expect(flatten(parseMath("\\wobble{x}"))).toBe("\\wobblex");
    expect(() => parseMath("\\frac{")).not.toThrow();
    expect(() => parseMath("^^^{{{")).not.toThrow();
    expect(() => parseMath("\\begin{pmatrix} a")).not.toThrow();
  });

  it("reads text set in words", () => {
    const [styled] = parseMath("\\text{cost per token}").body;
    expect(styled.style).toBe("upright");
    expect(flatten(styled)).toBe("cost per token");
  });
});

describe("finding maths in a message", () => {
  const kinds = (nodes) => nodes.map((node) => node.type);

  it("reads a formula between single dollars", () => {
    const nodes = parseInline("the cost is $c = n p$ per turn");
    expect(kinds(nodes)).toEqual(["text", "math", "text"]);
    expect(nodes[1].value).toBe("c = n p");
  });

  it("leaves prices alone", () => {
    expect(kinds(parseInline("it costs $5 and then $10 more"))).toEqual(["text"]);
    expect(kinds(parseInline("$ x $ is not maths either"))).toEqual(["text"]);
    expect(kinds(parseInline("a lone $ sign"))).toEqual(["text"]);
  });

  it("reads the bracket spelling too", () => {
    const nodes = parseInline("so \\(a^2 + b^2\\) holds");
    expect(kinds(nodes)).toEqual(["text", "math", "text"]);
    expect(nodes[1].value).toBe("a^2 + b^2");
  });

  it("does not read across a line break", () => {
    expect(kinds(parseInline("$a\nb$"))).toEqual(["text", "break", "text"]);
  });

  it("reads a display formula as its own block", () => {
    const blocks = parseBlocks("before\n\n$$\n\\frac{a}{b}\n$$\n\nafter");
    expect(blocks.map((block) => block.type)).toEqual(["paragraph", "math", "paragraph"]);
    expect(blocks[1].value).toBe("\\frac{a}{b}");
  });

  it("reads a display formula written on one line", () => {
    const blocks = parseBlocks("$$E = mc^2$$");
    expect(blocks).toEqual([{ type: "math", value: "E = mc^2" }]);
  });

  it("reads the bracket spelling of a display formula", () => {
    const blocks = parseBlocks("\\[\n  x = 1\n\\]");
    expect(blocks).toEqual([{ type: "math", value: "x = 1" }]);
  });

  it("waits for the closing delimiter while a reply is streaming", () => {
    const streaming = parseBlocks("$$\n\\frac{a}{b", { streaming: true });
    expect(streaming.map((block) => block.type)).toEqual(["paragraph"]);
    // The same text, finished, is a formula.
    const done = parseBlocks("$$\n\\frac{a}{b\n$$", { streaming: true });
    expect(done.map((block) => block.type)).toEqual(["math"]);
  });

  it("does not mistake a table row full of dollars for a formula", () => {
    const blocks = parseBlocks("| cost |\n| --- |\n| $5 |");
    expect(blocks[0].type).toBe("table");
  });
});
