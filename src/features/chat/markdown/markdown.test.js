import { describe, expect, it } from "vitest";
import { parseBlocks } from "./parse-blocks.js";
import { parseInline } from "./parse-inline.js";

/** Flattens an inline tree to the text a reader would see. */
function text(nodes) {
  return nodes
    .map((node) => {
      if (node.type === "text" || node.type === "code") return node.value;
      if (node.type === "break") return "\n";
      if (node.type === "image") return node.alt;
      return text(node.children);
    })
    .join("");
}

function types(blocks) {
  return blocks.map((block) => block.type);
}

/** Feeds a string one character at a time, collecting the parse at each step. */
function stream(source) {
  const frames = [];
  for (let i = 1; i <= source.length; i += 1)
    frames.push(parseBlocks(source.slice(0, i), { streaming: true }));
  return frames;
}

describe("blocks", () => {
  it("reads all six ATX heading levels and ignores closing hashes", () => {
    const blocks = parseBlocks("# one\n\n###### six\n\n## two ##");
    expect(blocks).toEqual([
      { type: "heading", level: 1, text: "one" },
      { type: "heading", level: 6, text: "six" },
      { type: "heading", level: 2, text: "two" },
    ]);
    // seven hashes is not a heading
    expect(types(parseBlocks("####### nope"))).toEqual(["paragraph"]);
  });

  it("reads setext underlines", () => {
    expect(parseBlocks("Title\n===")).toEqual([{ type: "heading", level: 1, text: "Title" }]);
    expect(parseBlocks("Title\n---")).toEqual([{ type: "heading", level: 2, text: "Title" }]);
    // with a blank line above it, the same rule is a thematic break
    expect(types(parseBlocks("Title\n\n---"))).toEqual(["paragraph", "hr"]);
  });

  it("keeps soft line breaks inside a paragraph and splits on a blank line", () => {
    const blocks = parseBlocks("one\ntwo\n\nthree");
    expect(blocks).toEqual([
      { type: "paragraph", text: "one\ntwo" },
      { type: "paragraph", text: "three" },
    ]);
  });

  it("reads every horizontal rule spelling", () => {
    expect(types(parseBlocks("---\n\n***\n\n___\n\n- - -"))).toEqual(["hr", "hr", "hr", "hr"]);
  });

  it("fences with a language, with tildes, and indented inside a list", () => {
    const [block] = parseBlocks("```js\nconst a = 1;\n```");
    expect(block).toEqual({ type: "code", lang: "js", code: "const a = 1;", closed: true });

    const [tilde] = parseBlocks("~~~python\nx = 1\n~~~");
    expect(tilde.lang).toBe("python");
    expect(tilde.code).toBe("x = 1");

    // a ``` inside a ~~~ fence is content, not a closer
    const [mixed] = parseBlocks("~~~\n```\n~~~");
    expect(mixed.code).toBe("```");

    const [list] = parseBlocks("- step\n\n  ```sh\n  npm run dev\n  ```");
    expect(list.type).toBe("list");
    const nested = list.items[0].blocks;
    expect(types(nested)).toEqual(["paragraph", "code"]);
    expect(nested[1]).toMatchObject({ lang: "sh", code: "npm run dev" });
  });

  it("reads a four-space indented block as code", () => {
    const [block] = parseBlocks("    line one\n      line two");
    expect(block).toEqual({ type: "code", lang: "", code: "line one\n  line two", closed: true });
  });

  it("does not turn an indented continuation of a paragraph into code", () => {
    expect(types(parseBlocks("a sentence\n    that wrapped"))).toEqual(["paragraph"]);
  });
});

describe("tables", () => {
  it("reads a pipe table with alignment", () => {
    const [table] = parseBlocks("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |");
    expect(table.type).toBe("table");
    expect(table.head).toEqual(["a", "b", "c"]);
    expect(table.align).toEqual(["left", "center", "right"]);
    expect(table.rows).toEqual([
      ["1", "2", "3"],
      ["4", "5", "6"],
    ]);
    expect(table.partial).toBe(false);
  });

  it("accepts rows without outer pipes and pads short rows", () => {
    const [table] = parseBlocks("a | b\n--- | ---\n1 | 2 | 3\n4");
    expect(table.head).toEqual(["a", "b"]);
    expect(table.rows).toEqual([["1", "2"]]);
    // a row with no pipe at all ends the table
    expect(table.rows.length).toBe(1);
  });

  it("keeps an escaped pipe inside a cell", () => {
    const [table] = parseBlocks("| a | b |\n|---|---|\n| x \\| y | z |");
    expect(table.rows[0]).toEqual(["x | y", "z"]);
  });

  it("does not read a setext heading containing a pipe as a table", () => {
    expect(types(parseBlocks("a | b\n---"))).toEqual(["heading"]);
  });

  it("stops the table at a blank line", () => {
    const blocks = parseBlocks("| a |\n|---|\n| 1 |\n\nafter");
    expect(types(blocks)).toEqual(["table", "paragraph"]);
  });
});

describe("lists", () => {
  it("reads unordered and ordered markers and remembers the start number", () => {
    expect(parseBlocks("- a\n- b")[0]).toMatchObject({ ordered: false, tight: true });
    expect(parseBlocks("3. a\n4. b")[0]).toMatchObject({ ordered: true, start: 3 });
    // a different marker family starts a new list
    expect(types(parseBlocks("- a\n1. b"))).toEqual(["list", "list"]);
  });

  it("nests by indentation", () => {
    const [list] = parseBlocks("- one\n  - inner\n    - deepest\n- two");
    expect(list.items).toHaveLength(2);
    const inner = list.items[0].blocks;
    expect(types(inner)).toEqual(["paragraph", "list"]);
    const deepest = inner[1].items[0].blocks;
    expect(types(deepest)).toEqual(["paragraph", "list"]);
    expect(deepest[1].items[0].blocks[0].text).toBe("deepest");
  });

  it("marks a list loose when a blank line separates items", () => {
    expect(parseBlocks("- a\n- b")[0].tight).toBe(true);
    expect(parseBlocks("- a\n\n- b")[0].tight).toBe(false);
    // a blank line before unrelated prose does not loosen the list
    expect(parseBlocks("- a\n- b\n\ntail")[0].tight).toBe(true);
  });

  it("gives a loose item its paragraphs", () => {
    const [list] = parseBlocks("- first\n\n  second\n\n- next");
    expect(list.tight).toBe(false);
    expect(types(list.items[0].blocks)).toEqual(["paragraph", "paragraph"]);
  });

  it("reads task list markers", () => {
    const [list] = parseBlocks("- [ ] todo\n- [x] done\n- plain");
    expect(list.items.map((item) => item.checked)).toEqual([false, true, null]);
    expect(list.items[1].blocks[0].text).toBe("done");
  });

  it("takes a lazy continuation line into the item", () => {
    const [list] = parseBlocks("- one\ncontinued\n- two");
    expect(list.items[0].blocks[0].text).toBe("one\ncontinued");
    expect(list.items).toHaveLength(2);
  });
});

describe("quotes", () => {
  it("holds multiple paragraphs and nested quotes", () => {
    const [quote] = parseBlocks("> one\n>\n> two\n>\n> > deep");
    expect(quote.type).toBe("quote");
    expect(types(quote.blocks)).toEqual(["paragraph", "paragraph", "quote"]);
    expect(quote.blocks[2].blocks[0].text).toBe("deep");
  });

  it("holds a list and a code fence", () => {
    const [quote] = parseBlocks("> - a\n> - b\n>\n> ```\n> code\n> ```");
    expect(types(quote.blocks)).toEqual(["list", "code"]);
    expect(quote.blocks[1].code).toBe("code");
  });

  it("ends at a blank line", () => {
    expect(types(parseBlocks("> quoted\n\nplain"))).toEqual(["quote", "paragraph"]);
  });
});

describe("inline", () => {
  it("reads bold, italic, bold italic and strikethrough", () => {
    expect(parseInline("**b**")[0]).toMatchObject({ type: "strong" });
    expect(parseInline("*i*")[0]).toMatchObject({ type: "em" });
    expect(parseInline("~~s~~")[0]).toMatchObject({ type: "del" });
    const [both] = parseInline("***x***");
    expect(both.type).toBe("strong");
    expect(both.children[0].type).toBe("em");
    expect(text(both.children)).toBe("x");
  });

  it("leaves intraword underscores alone", () => {
    expect(parseInline("snake_case_name")).toEqual([{ type: "text", value: "snake_case_name" }]);
    expect(parseInline("_real_")[0].type).toBe("em");
  });

  it("does not open emphasis against whitespace", () => {
    expect(parseInline("2 * 3 * 4").every((node) => node.type === "text")).toBe(true);
  });

  it("reads code spans, including a double backtick span holding a backtick", () => {
    expect(parseInline("`x`")[0]).toEqual({ type: "code", value: "x" });
    expect(parseInline("`` a ` b ``")[0]).toEqual({ type: "code", value: "a ` b" });
    // markers inside a code span stay literal
    expect(parseInline("`**not bold**`")[0]).toEqual({ type: "code", value: "**not bold**" });
  });

  it("does not let a code span hide an emphasis closer", () => {
    const [strong] = parseInline("**a `b` c**");
    expect(strong.type).toBe("strong");
    expect(text(strong.children)).toBe("a b c");
  });

  it("reads links, angle autolinks and bare autolinks", () => {
    expect(parseInline("[l](https://x.dev)")[0]).toMatchObject({
      type: "link",
      href: "https://x.dev",
    });
    expect(parseInline('[l](https://x.dev "title")')[0].href).toBe("https://x.dev");
    expect(parseInline("<https://x.dev>")[0]).toMatchObject({ type: "link", href: "https://x.dev" });
    const [, link] = parseInline("see https://x.dev/a_b.");
    expect(link).toMatchObject({ type: "link", href: "https://x.dev/a_b" });
    expect(parseInline("www.x.dev")[0].href).toBe("https://www.x.dev");
  });

  it("renders an image as an image node, never as fetched markup", () => {
    expect(parseInline("![alt](https://x.dev/a.png)")[0]).toEqual({
      type: "image",
      alt: "alt",
      src: "https://x.dev/a.png",
    });
  });

  it("honours backslash escapes", () => {
    expect(parseInline("\\*not bold\\*")).toEqual([{ type: "text", value: "*not bold*" }]);
    expect(parseInline("\\[not a link](x)")[0].value.startsWith("[not a link]")).toBe(true);
    expect(parseInline("a\\\\b")).toEqual([{ type: "text", value: "a\\b" }]);
  });

  it("never emits raw markup for angle brackets", () => {
    const nodes = parseInline("<script>alert(1)</script>");
    expect(nodes.every((node) => node.type === "text")).toBe(true);
    expect(text(nodes)).toBe("<script>alert(1)</script>");
  });
});

describe("streaming", () => {
  it("treats an unterminated fence as a code block from the opening fence on", () => {
    const source = "```js\nconst a = 1;\nconst b = 2;\n```";
    const frames = stream(source);
    // every frame from the first newline after the fence is a single code block
    const fromFence = frames.slice(source.indexOf("\n"));
    for (const blocks of fromFence) expect(types(blocks)).toEqual(["code"]);
    expect(frames[frames.length - 1][0].closed).toBe(true);
    expect(frames[frames.length - 2][0].closed).toBe(false);
  });

  it("keeps a partly typed fence body out of the prose", () => {
    const [block] = parseBlocks("```py\nprint(", { streaming: true });
    expect(block).toMatchObject({ type: "code", lang: "py", code: "print(", closed: false });
  });

  it("never flashes a table header as a paragraph of pipes", () => {
    const source = "| name | size |\n|---|---|\n| a | 1 |";
    for (const blocks of stream(source)) {
      const kinds = new Set(types(blocks));
      expect(kinds.has("paragraph")).toBe(false);
    }
    const done = parseBlocks(source, { streaming: true });
    expect(done[0].partial).toBe(false);
    expect(done[0].rows).toEqual([["a", "1"]]);
  });

  it("marks the header-only table as partial and keeps its columns", () => {
    const head = parseBlocks("| name | size |\n", { streaming: true })[0];
    expect(head).toMatchObject({ type: "table", partial: true, head: ["name", "size"], rows: [] });
    const half = parseBlocks("| name | size |\n|--", { streaming: true })[0];
    expect(half.head).toEqual(["name", "size"]);
    expect(half.partial).toBe(true);
  });

  it("leaves a pipe in ordinary prose alone", () => {
    expect(types(parseBlocks("use a | b to pipe", { streaming: true }))).toEqual(["paragraph"]);
    expect(types(parseBlocks("text\n| a | b |", { streaming: true }))).toEqual(["paragraph"]);
  });

  it("does not let a half-written link swallow the rest of the message", () => {
    for (const partial of ["[label", "[label]", "[label](", "[label](htt"]) {
      const nodes = parseInline(`${partial} and more text`);
      expect(nodes.some((node) => node.type === "link")).toBe(false);
      expect(text(nodes)).toBe(`${partial} and more text`);
    }
    const done = parseInline("[label](https://x.dev) and more text");
    expect(done[0]).toMatchObject({ type: "link", href: "https://x.dev" });
    expect(text(done)).toBe("label and more text");
  });

  it("renders trailing incomplete inline markers as plain text", () => {
    for (const partial of ["**", "**bo", "*i", "~~s", "`code", "``a`"]) {
      const nodes = parseInline(`done ${partial}`);
      expect(text(nodes)).toBe(`done ${partial}`);
      expect(nodes.every((node) => node.type === "text")).toBe(true);
    }
  });

  it("only changes an inline run's form when its closer lands", () => {
    const source = "a **bold** tail";
    const forms = stream(source).map((blocks) => blocks[0].text);
    // the block never stops being a paragraph while the emphasis completes
    expect(new Set(stream(source).map((blocks) => blocks[0].type))).toEqual(new Set(["paragraph"]));
    expect(forms[forms.length - 1]).toBe(source);
  });

  it("keeps every block above the cursor byte-identical as text arrives", () => {
    const source = "# Title\n\n- a\n- b\n\n```js\nx\n```\n\nTail parag";
    const frames = stream(source);
    const last = frames[frames.length - 1];
    const earlier = frames[frames.length - 2];
    // only the final block differs between the last two frames
    expect(JSON.stringify(last.slice(0, -1))).toBe(JSON.stringify(earlier.slice(0, -1)));
  });
});

/**
 * The markup a model actually writes in a message.
 *
 * Not a licence to render HTML: every tag below becomes one of the node types
 * this file already had, and anything not on the list stays the characters it
 * was written as. The one that mattered is `<img>`, which models write about
 * as often as they write the markdown form.
 */
describe("inline HTML", () => {
  it("reads an image tag, with the width it asked for", () => {
    const nodes = parseInline('<img src="https://x.dev/a.png" alt="A chart" width="600"/>');
    expect(nodes).toEqual([
      { type: "image", src: "https://x.dev/a.png", alt: "A chart", width: 600 },
    ]);
  });

  it("reads an image tag written without the closing slash", () => {
    // A path with a drive letter rather than a leading slash: the test that
    // hunts for absolute asset paths in this repo's own markup reads every
    // file, and a fixture is indistinguishable from a mistake to it.
    const [node] = parseInline('<img src="D:/shots/shot.png">');
    expect(node).toMatchObject({ type: "image", src: "D:/shots/shot.png" });
  });

  it("turns the text tags into the nodes they mean", () => {
    expect(parseInline("<b>a</b>")[0].type).toBe("strong");
    expect(parseInline("<i>a</i>")[0].type).toBe("em");
    expect(parseInline("<del>a</del>")[0].type).toBe("del");
    expect(parseInline("<code>a &lt; b</code>")[0]).toEqual({ type: "code", value: "a &lt; b" });
    expect(parseInline("<br>")[0]).toEqual({ type: "break" });
  });

  it("keeps a tag it does not know as the text it was", () => {
    expect(parseInline("<script>alert(1)</script>")).toEqual([
      { type: "text", value: "<script>alert(1)</script>" },
    ]);
    expect(parseInline("<div>x</div>")[0].type).toBe("text");
  });

  it("leaves a half-typed tag alone until it is finished", () => {
    expect(parseInline('here <img src="htt')).toEqual([
      { type: "text", value: 'here <img src="htt' },
    ]);
    expect(parseInline("<b>unclosed")).toEqual([{ type: "text", value: "<b>unclosed" }]);
  });

  it("reads a link written as a tag", () => {
    const [node] = parseInline('<a href="https://x.dev">go</a>');
    expect(node).toMatchObject({ type: "link", href: "https://x.dev" });
    expect(node.children).toEqual([{ type: "text", value: "go" }]);
  });
});

describe("footnotes", () => {
  it("reads a reference and the note it points at", () => {
    const blocks = parseBlocks("Some claim[^1].\n\n[^1]: Where it came from.");
    expect(blocks[0].type).toBe("paragraph");
    expect(blocks[1]).toMatchObject({ type: "footnote", id: "1" });
    expect(blocks[1].blocks[0].text).toBe("Where it came from.");

    const nodes = parseInline("Some claim[^1].");
    expect(nodes[1]).toEqual({ type: "footref", id: "1" });
  });

  it("reads several notes written in a row", () => {
    const blocks = parseBlocks("[^a]: First.\n[^b]: Second.");
    expect(blocks.map((block) => block.id)).toEqual(["a", "b"]);
  });

  it("takes the lines under a note as part of it", () => {
    const [note] = parseBlocks("[^1]: First line\n  and its continuation.");
    expect(note.blocks[0].text).toBe("First line\nand its continuation.");
  });

  it("does not read an ordinary link as a reference", () => {
    expect(parseInline("[label](https://x.dev)")[0].type).toBe("link");
    expect(parseInline("[^ ]")).toEqual([{ type: "text", value: "[^ ]" }]);
  });
});

/**
 * The model's dashes.
 *
 * Every model reaches for the em dash, three to a paragraph, and it reads on
 * screen as exactly what it is. Prose gets a spaced hyphen instead; anything
 * that is not prose - code, an address - keeps what was written.
 */
describe("mentions", () => {
  it("reads @handle as a mention and keeps the punctuation after it outside", () => {
    expect(parseInline("over to you, @inertia-dev, and @nova.")).toEqual([
      { type: "text", value: "over to you, " },
      { type: "mention", handle: "inertia-dev" },
      { type: "text", value: ", and " },
      { type: "mention", handle: "nova" },
      { type: "text", value: "." },
    ]);
  });

  it("starts a message with one", () => {
    expect(parseInline("@nova hi")[0]).toEqual({ type: "mention", handle: "nova" });
  });

  it("does not read an email address as a mention", () => {
    expect(parseInline("mail nick@nova.dev")).toEqual([{ type: "text", value: "mail nick@nova.dev" }]);
  });

  it("leaves a bare @ alone", () => {
    expect(parseInline("at @ noon")).toEqual([{ type: "text", value: "at @ noon" }]);
  });

  it("leaves a handle inside code alone", () => {
    expect(parseInline("`@nova`")).toEqual([{ type: "code", value: "@nova" }]);
  });
});

describe("em dashes in prose", () => {
  it("becomes a spaced hyphen, whether or not it was spaced", () => {
    expect(parseInline("one — two")).toEqual([{ type: "text", value: "one - two" }]);
    expect(parseInline("one—two")).toEqual([{ type: "text", value: "one - two" }]);
  });

  it("keeps a range as a range", () => {
    expect(parseInline("2019–2021")).toEqual([{ type: "text", value: "2019-2021" }]);
  });

  it("leaves code alone", () => {
    const [code] = parseInline("`a — b`");
    expect(code.type).toBe("code");
    expect(code.value).toContain("—");
  });
});
