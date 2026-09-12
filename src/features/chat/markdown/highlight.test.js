import { describe, expect, it } from "vitest";
import { languageOf, tokenize } from "./highlight.js";

/** The spans of one kind, in order, as plain strings. */
const of = (spans, kind) => spans.filter((span) => span.kind === kind).map((span) => span.text);
const joined = (spans) => spans.map((span) => span.text).join("");

describe("what counts as a language", () => {
  it("knows the names people actually write on a fence", () => {
    expect(languageOf("javascript")).toBe("js");
    expect(languageOf("TSX")).toBe("ts");
    expect(languageOf("bash")).toBe("shell");
    expect(languageOf("yml")).toBe("yaml");
    expect(languageOf("svg")).toBe("html");
    expect(languageOf("patch")).toBe("diff");
  });

  it("says no to one it does not know, rather than guessing", () => {
    expect(languageOf("mermaid")).toBe(null);
    expect(languageOf("")).toBe(null);
    expect(languageOf(undefined)).toBe(null);
  });
});

/**
 * The invariant that matters more than any colour: what is shown is what was
 * written. A highlighter that drops a character is worse than none, because
 * the reader copies what they see.
 */
describe("nothing is ever lost", () => {
  const samples = [
    ["js", "const a = `t${x}` // 'unclosed\nlet b = /re/g;"],
    ["python", 'def f(x):\n  """doc\n  more"""\n  return {**x, "k": 1}'],
    ["json", '{"a": [1, 2.5e3, null], "b": {"c": true}}'],
    ["html", '<div class="a" data-x=1><!-- hi --><br/>text &amp; more</div>'],
    ["css", ".a > b:hover { color: #fff; margin: 0 auto; }\n@media (min-width: 10px) { }"],
    ["yaml", "a:\n  - b: 1 # note\n  - c: 'x'\n---\nd: true"],
    ["diff", "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n context"],
    ["shell", 'for f in *.txt; do echo "$f" ${HOME}; done # loop'],
    ["ini", "[section]\nkey = value ; note\nn = 42"],
    ["sql", "SELECT a, b FROM t WHERE x = 'y' -- note"],
    ["go", "func main() {\n\tfmt.Println(`raw`)\n}"],
    ["rust", 'fn main() { let x: u32 = 0xff; println!("{x}"); }'],
    ["markdown", "# Title\n\n- one\n> quote\n```js\nx\n```"],
    ["nonsense-language", "whatever <> ```"],
    ["js", ""],
    ["js", "\n\n\n"],
    ["js", "unterminated /* comment"],
    ["python", "s = '''never closed"],
  ];

  for (const [lang, code] of samples) {
    it(`${lang}: ${JSON.stringify(code.slice(0, 24))}`, () => {
      expect(joined(tokenize(code, lang))).toBe(code);
    });
  }
});

describe("javascript", () => {
  const code = [
    "// a note",
    "import { thing } from './where.js';",
    "const n = 42, big = 0xff;",
    "export function greet(name) {",
    "  return `hi ${name}`; /* done */",
    "}",
  ].join("\n");
  const spans = tokenize(code, "js");

  it("finds comments in both shapes", () => {
    expect(of(spans, "comment")).toEqual(["// a note", "/* done */"]);
  });

  it("finds strings, including a template that spans an interpolation", () => {
    expect(of(spans, "string")).toEqual(["'./where.js'", "`hi ${name}`"]);
  });

  it("finds keywords and numbers", () => {
    expect(of(spans, "keyword")).toEqual(
      expect.arrayContaining(["import", "from", "const", "export", "function", "return"])
    );
    expect(of(spans, "number")).toEqual(["42", "0xff"]);
  });

  it("marks a name that is being called", () => {
    expect(of(spans, "function")).toContain("greet");
  });

  it("does not let an apostrophe in a comment swallow the file", () => {
    const spans = tokenize("// it's fine\nconst x = 1;", "js");
    expect(of(spans, "keyword")).toContain("const");
    expect(of(spans, "number")).toEqual(["1"]);
  });
});

describe("python", () => {
  const spans = tokenize('@dec\ndef f(a=1):\n    """doc"""\n    return None  # end', "python");

  it("keeps a triple-quoted string in one piece", () => {
    expect(of(spans, "string")).toEqual(['"""doc"""']);
  });

  it("marks a decorator, a keyword and a comment", () => {
    expect(of(spans, "meta")).toEqual(["@dec"]);
    expect(of(spans, "keyword")).toEqual(expect.arrayContaining(["def", "return", "None"]));
    expect(of(spans, "comment")).toEqual(["# end"]);
  });
});

describe("json", () => {
  const spans = tokenize('{\n  "name": "inertia",\n  "count": 2,\n  "ok": true\n}', "json");

  it("tells a key from a value", () => {
    expect(of(spans, "property")).toEqual(['"name"', '"count"', '"ok"']);
    expect(of(spans, "string")).toEqual(['"inertia"']);
    expect(of(spans, "keyword")).toEqual(["true"]);
  });
});

describe("html", () => {
  const spans = tokenize('<a href="/x" class=\'y\'>text</a>', "html");

  it("separates the tag, its attributes and its values", () => {
    expect(of(spans, "tag")).toEqual(["a", "a"]);
    expect(of(spans, "attribute")).toEqual(["href", "class"]);
    expect(of(spans, "string")).toEqual(['"/x"', "'y'"]);
    expect(of(spans, "plain")).toContain("text");
  });
});

describe("css", () => {
  const spans = tokenize(".card { color: #fff; padding: 4px }", "css");

  it("tells a selector from a property from a value", () => {
    expect(of(spans, "tag")).toEqual([".card"]);
    expect(of(spans, "property")).toEqual(["color", "padding"]);
    expect(of(spans, "number")).toEqual(["#fff", "4px"]);
  });
});

describe("diff", () => {
  const spans = tokenize("@@ -1,2 +1,2 @@\n-gone\n+here\n same", "diff");

  it("colours the two sides and the file headers", () => {
    expect(of(spans, "removed")).toEqual(["-gone"]);
    expect(of(spans, "added")).toEqual(["+here"]);
    expect(of(spans, "meta")).toEqual(["@@ -1,2 +1,2 @@"]);
  });
});

describe("yaml", () => {
  const spans = tokenize("name: inertia\ncount: 3\nflag: true\n# note", "yaml");

  it("marks keys, and reads the value's shape", () => {
    expect(of(spans, "property")).toEqual(["name", "count", "flag"]);
    expect(of(spans, "number")).toEqual([" 3"]);
    expect(of(spans, "keyword")).toEqual([" true"]);
    expect(of(spans, "comment")).toEqual(["# note"]);
  });
});

describe("an unknown language", () => {
  it("comes back as one plain span, so the block renders as it always did", () => {
    const spans = tokenize("graph TD\n  A-->B", "mermaid");
    expect(spans).toEqual([{ kind: "plain", text: "graph TD\n  A-->B" }]);
  });
});
