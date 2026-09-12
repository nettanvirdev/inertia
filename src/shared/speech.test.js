import { describe, it, expect } from "vitest";
import { MAX_SPEAK_CHARS, speakable, speechFor, toUtterances } from "./speech";

/**
 * Speech is the one output nobody can skim, so a bad rule here is not a cosmetic
 * bug - it is a minute of someone's life spent listening to punctuation. The
 * cases below are the shapes an agent's reply actually takes.
 */

describe("speakable", () => {
  it("names the language of a fenced block instead of reading it", () => {
    const markdown = [
      "Here is the fix.",
      "```ts",
      "export const x = 1;",
      "if (a < b) { throw new Error('no'); }",
      "```",
      "That should do it.",
    ].join("\n");

    expect(speakable(markdown)).toBe(
      "Here is the fix. (a TypeScript code block) That should do it.",
    );
  });

  it("falls back to a bare placeholder when the fence declares nothing", () => {
    expect(speakable("```\nsome code\n```")).toBe("(a code block)");
  });

  it("says an unknown fence tag as written rather than staying silent", () => {
    expect(speakable("```nim\nlet x = 1\n```")).toBe("(a nim code block)");
  });

  it("keeps inline code but drops the backticks", () => {
    expect(speakable("Call `refresh()` twice.")).toBe("Call refresh() twice.");
  });

  it("ends a heading so it does not run into the next sentence", () => {
    expect(speakable("## Setup\nRun the installer.")).toBe("Setup. Run the installer.");
  });

  it("leaves a heading alone when it already ends in punctuation", () => {
    expect(speakable("# Ready?\nYes.")).toBe("Ready? Yes.");
  });

  it("drops list markers, including from a nested list", () => {
    const markdown = ["- First item", "  - Nested item", "* Second item", "1. Third item"].join(
      "\n",
    );
    expect(speakable(markdown)).toBe("First item Nested item Second item Third item");
  });

  it("reads a link as its text and a bare URL as a link", () => {
    expect(speakable("See [the docs](https://example.com/a/b/c) for more.")).toBe(
      "See the docs for more.",
    );
    expect(speakable("See https://example.com/a/b?q=1 for more.")).toBe(
      "See (a link) for more.",
    );
  });

  it("reads an image as its alt text, or says there was one", () => {
    expect(speakable("![the login screen](shot.png)")).toBe("the login screen");
    expect(speakable("![](shot.png)")).toBe("(an image)");
  });

  it("collapses a table to one sentence per row and drops the separator", () => {
    const markdown = [
      "| Name | Status |",
      "| --- | --- |",
      "| build | passing |",
      "| lint | failing |",
    ].join("\n");

    expect(speakable(markdown)).toBe("Name, Status. build, passing. lint, failing.");
  });

  it("strips emphasis markers and keeps the words", () => {
    expect(speakable("That is **really** _quite_ ~~bad~~ *urgent*.")).toBe(
      "That is really quite bad urgent.",
    );
  });

  it("leaves an underscore that is part of an identifier alone", () => {
    expect(speakable("The flag is auto_save_enabled today.")).toBe(
      "The flag is auto_save_enabled today.",
    );
  });

  it("drops blockquote markers", () => {
    expect(speakable("> He said no.\n> Twice.")).toBe("He said no. Twice.");
  });

  it("drops emoji", () => {
    expect(speakable("Shipped 🚀 and tested 🎉.")).toBe("Shipped and tested .");
  });

  it("shortens a deep path to its filename", () => {
    expect(speakable("Look at apps/web/src/lib/dictation.ts for the answer.")).toBe(
      "Look at dictation.ts for the answer.",
    );
  });

  it("leaves a shallow path and anything with spaces alone", () => {
    expect(speakable("The ratio is src/lib and 3 / 4 of the way in.")).toBe(
      "The ratio is src/lib and 3 / 4 of the way in.",
    );
  });

  it("collapses blank lines and never doubles a space", () => {
    const result = speakable("One.\n\n\n\nTwo.\n\n   \n\nThree.");
    expect(result).toBe("One. Two. Three.");
    expect(result).not.toMatch(/ {2}/);
  });

  it("returns nothing for empty or whitespace input", () => {
    expect(speakable("")).toBe("");
    expect(speakable("   \n\n  \t ")).toBe("");
    expect(speakable(null)).toBe("");
    expect(speakable(undefined)).toBe("");
  });
});

describe("toUtterances", () => {
  it("returns nothing for empty input", () => {
    expect(toUtterances("")).toEqual([]);
    expect(toUtterances("   ")).toEqual([]);
    expect(toUtterances(null)).toEqual([]);
  });

  it("splits on sentence boundaries", () => {
    expect(toUtterances("The build passed. The lint job failed. Nothing else changed.")).toEqual([
      "The build passed.",
      "The lint job failed.",
      "Nothing else changed.",
    ]);
  });

  it("keeps questions and exclamations whole", () => {
    expect(toUtterances("Did the build pass? It did not pass! Try again now.")).toEqual([
      "Did the build pass?",
      "It did not pass!",
      "Try again now.",
    ]);
  });

  it("does not end a sentence on an abbreviation", () => {
    expect(
      toUtterances("Some formats, e.g. YAML and TOML, are supported by the parser."),
    ).toEqual(["Some formats, e.g. YAML and TOML, are supported by the parser."]);

    expect(toUtterances("Prefer a queue vs. a raw array in this hot path.")).toEqual([
      "Prefer a queue vs. a raw array in this hot path.",
    ]);

    expect(toUtterances("Timeouts, retries, etc. are configured in one place.")).toEqual([
      "Timeouts, retries, etc. are configured in one place.",
    ]);
  });

  it("treats a lone capital as an initial rather than a full stop", () => {
    expect(toUtterances("The report from J. R. Baker landed this morning.")).toEqual([
      "The report from J. R. Baker landed this morning.",
    ]);
  });

  it("does not split a decimal number", () => {
    expect(toUtterances("The request took 3.5 seconds to come back from cache.")).toEqual([
      "The request took 3.5 seconds to come back from cache.",
    ]);
  });

  it("glues a short fragment onto the sentence before it", () => {
    const result = toUtterances("The migration finished cleanly. Good.");
    expect(result).toEqual(["The migration finished cleanly. Good."]);
  });

  it("lets a short fragment stand alone when there is nothing before it", () => {
    expect(toUtterances("Done.")).toEqual(["Done."]);
  });

  it("respects a caller's minChars", () => {
    expect(toUtterances("A short one. And another short one.", { minChars: 1 })).toEqual([
      "A short one.",
      "And another short one.",
    ]);
  });

  it("splits an over-long sentence at a clause boundary", () => {
    const sentence =
      "The parser walks the token stream once and records every span it sees, " +
      "and then a second pass resolves the references it could not resolve on the first";
    const result = toUtterances(sentence, { maxChars: 90 });

    expect(result.length).toBeGreaterThan(1);
    for (const part of result) expect(part.length).toBeLessThanOrEqual(90);
    expect(result.join(" ").replace(/\s+/g, " ")).toBe(sentence);
  });

  it("falls back to a word boundary and never splits mid-word", () => {
    const sentence = `${"alpha ".repeat(30)}omega`.trim();
    const result = toUtterances(sentence, { maxChars: 40 });

    for (const part of result) {
      expect(part.length).toBeLessThanOrEqual(40);
      expect(part.split(/\s+/).every((word) => word === "alpha" || word === "omega")).toBe(true);
    }
  });
});

describe("speechFor", () => {
  it("composes the two halves", () => {
    const markdown = ["# Result", "", "The suite is green.", "", "```js", "run();", "```"].join(
      "\n",
    );

    expect(speechFor(markdown)).toEqual([
      "Result.",
      "The suite is green.",
      "(a JavaScript code block)",
    ]);
  });

  it("returns nothing for a reply with nothing speakable in it", () => {
    expect(speechFor("")).toEqual([]);
    expect(speechFor("   \n\n")).toEqual([]);
  });

  it("truncates at a word boundary within the cap", () => {
    const markdown = "Wolverine badger otter. ".repeat(300);
    const result = speechFor(markdown);
    const total = result.join(" ").length;

    expect(total).toBeLessThanOrEqual(MAX_SPEAK_CHARS);
    expect(result.at(-1)).toMatch(/(Wolverine|badger|otter)\.?$/);
  });

  it("passes options through to the splitter", () => {
    const result = speechFor("One two three four five six seven eight nine ten.", {
      maxChars: 20,
    });
    for (const part of result) expect(part.length).toBeLessThanOrEqual(20);
  });
});
