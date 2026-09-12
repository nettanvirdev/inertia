import { describe, expect, it } from "vitest";
import {
  attachmentUrl,
  cardOf,
  classify,
  columnsOf,
  fitsATable,
  isImageUrl,
  isUrl,
  labelOf,
  leadOf,
  parseResult,
  preview,
  roleOf,
  toneOf,
} from "./mcp-result.js";

describe("parseResult", () => {
  it("reads one document", () => {
    expect(parseResult('{"a":1}')).toEqual({ a: 1 });
    expect(parseResult("  [1,2]  ")).toEqual([1, 2]);
  });

  it("ignores the truncation trailer the flattener adds", () => {
    expect(parseResult('{"a":1}\n[result truncated]')).toEqual({ a: 1 });
  });

  it("reads a run of documents, one per line", () => {
    expect(parseResult('{"a":1}\n{"a":2}')).toEqual([{ a: 1 }, { a: 2 }]);
  });

  it("leaves prose alone", () => {
    expect(parseResult("The weather in Dhaka is warm.")).toBeUndefined();
    expect(parseResult("")).toBeUndefined();
    expect(parseResult(undefined)).toBeUndefined();
  });

  it("does not treat a bare number or word as a document", () => {
    expect(parseResult("42")).toBeUndefined();
    expect(parseResult("true")).toBeUndefined();
  });

  it("gives up on half-JSON rather than guessing", () => {
    expect(parseResult('{"a":1}\nnot json')).toBeUndefined();
    expect(parseResult('{"a":')).toBeUndefined();
  });
});

describe("classify", () => {
  it("names the empties", () => {
    expect(classify(null)).toBe("empty");
    expect(classify("")).toBe("empty");
    expect(classify([])).toBe("empty");
    expect(classify({})).toBe("empty");
  });

  it("separates a field from a paragraph", () => {
    expect(classify("Dhaka")).toBe("scalar");
    expect(classify(7)).toBe("scalar");
    expect(classify("a".repeat(200))).toBe("text");
    expect(classify("two\nlines")).toBe("text");
  });

  it("knows a picture from a string", () => {
    expect(classify("https://example.com/cat.png")).toBe("image");
    expect(classify("https://example.com/cat")).toBe("scalar");
  });

  it("calls a list of like objects a table", () => {
    const rows = [
      { name: "a", size: 1 },
      { name: "b", size: 2 },
    ];
    expect(classify(rows)).toBe("table");
    expect(classify(["a", "b"])).toBe("list");
    expect(classify([{ a: 1 }, { z: 2 }])).toBe("list");
  });

  it("calls an object a record", () => {
    expect(classify({ a: 1 })).toBe("record");
  });
});

describe("roleOf", () => {
  it("reads the common names", () => {
    expect(roleOf("url", "https://x.com")).toBe("url");
    expect(roleOf("image_url", "https://x.com/a.png")).toBe("image");
    expect(roleOf("title", "Hello")).toBe("title");
    expect(roleOf("snippet", "words")).toBe("body");
    expect(roleOf("score", 0.4)).toBe("score");
    expect(roleOf("published_date", "2024-01-01")).toBe("time");
    expect(roleOf("uuid", "x")).toBe("id");
  });

  it("lets the value overrule the name", () => {
    expect(roleOf("url", 42)).toBe("field");
    expect(roleOf("image", { a: 1 })).toBe("field");
    expect(roleOf("title", { a: 1 })).toBe("field");
  });

  it("does not match a name that merely contains one", () => {
    expect(roleOf("curl_command", "curl x")).toBe("field");
    expect(roleOf("thumbnail_policy", "strict")).toBe("field");
  });
});

describe("urls and pictures", () => {
  it("recognises what can be opened", () => {
    expect(isUrl("https://a.com/b")).toBe(true);
    expect(isUrl("not a url")).toBe(false);
  });

  it("draws only what will actually draw", () => {
    expect(isImageUrl("data:image/png;base64,AAA")).toBe(true);
    expect(isImageUrl("https://a.com/b.jpg?w=200")).toBe(true);
    expect(isImageUrl("https://a.com/page")).toBe(false);
  });

  it("turns an attachment into something an img can take", () => {
    expect(attachmentUrl({ mimeType: "image/png", data: "AAA" })).toBe(
      "data:image/png;base64,AAA"
    );
    expect(attachmentUrl({ data: "https://a.com/b.png" })).toBe("https://a.com/b.png");
    expect(attachmentUrl(null)).toBe("");
  });
});

describe("columnsOf and fitsATable", () => {
  const rows = [
    { name: "a", size: 1 },
    { name: "b", size: 2 },
    { name: "c", size: 3 },
  ];

  it("keeps the keys the rows share", () => {
    expect(columnsOf(rows)).toEqual(["name", "size"]);
  });

  it("drops a key most rows do not have", () => {
    expect(columnsOf([...rows, { name: "d", size: 4, odd: true }])).toEqual(["name", "size"]);
  });

  it("ignores nested values, which have no cell", () => {
    expect(columnsOf([{ a: 1, deep: { x: 1 } }, { a: 2, deep: { x: 2 } }])).toEqual(["a"]);
  });

  it("refuses a table for rows that carry paragraphs", () => {
    expect(fitsATable(rows)).toBe(true);
    expect(
      fitsATable(rows.map((row) => ({ ...row, note: "x".repeat(200) })))
    ).toBe(false);
  });

  it("refuses a table when the rows hold much more than the columns show", () => {
    const fat = rows.map((row) => ({ ...row, a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7 }));
    expect(fitsATable(fat)).toBe(false);
  });
});

describe("cardOf", () => {
  it("sorts a search result into its parts", () => {
    const card = cardOf({
      title: "Inertia",
      url: "https://inertia.dev",
      image: "https://inertia.dev/cover.png",
      snippet: "An app.",
      score: 0.91,
      meta: { source: "web" },
      empty: "",
    });
    expect(card.title).toBe("Inertia");
    expect(card.url).toBe("https://inertia.dev");
    expect(card.image).toBe("https://inertia.dev/cover.png");
    expect(card.body).toBe("An app.");
    expect(card.fields).toEqual([["score", 0.91, "score"]]);
    expect(card.nested).toEqual([["meta", { source: "web" }]]);
  });

  it("keeps an unrecognised field rather than dropping it", () => {
    expect(cardOf({ wibble: 3 }).fields).toEqual([["wibble", 3, "field"]]);
  });
});

describe("words", () => {
  it("says a key out loud", () => {
    expect(labelOf("follow_up_questions")).toBe("Follow up questions");
    expect(labelOf("publishedDate")).toBe("Published date");
    expect(labelOf("")).toBe("");
  });

  it("says what is behind a fold", () => {
    expect(preview([1, 2, 3])).toBe("3 items");
    expect(preview([1])).toBe("1 item");
    expect(preview({ a: 1, b: 2 })).toBe("a, b");
    expect(preview({ a: 1, b: 2, c: 3, d: 4 })).toBe("4 fields");
    expect(preview("x".repeat(100)).endsWith("…")).toBe(true);
    expect(preview(null)).toBe("empty");
  });

  it("colours by meaning and leaves the rest alone", () => {
    expect(toneOf("status", "error")).toBe("danger");
    expect(toneOf("status", "ok")).toBe("success");
    expect(toneOf("status", "pending")).toBe("warning");
    expect(toneOf("score", 0.4)).toBe("info");
    expect(toneOf("ok", true)).toBe("success");
    expect(toneOf("name", "Dhaka")).toBe("neutral");
  });

  it("finds the one long field worth showing first", () => {
    expect(leadOf({ query: "q", answer: "x".repeat(200) })).toBe("answer");
    expect(leadOf({ query: "q", results: [] })).toBe("");
  });
});
