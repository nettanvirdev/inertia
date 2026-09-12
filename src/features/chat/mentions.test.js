import { describe, expect, it } from "vitest";
import {
  activeQuery,
  applyMention,
  caretInMention,
  findSpecRanges,
  matchMentions,
  mentionSpecs,
  mentionedIds,
  slugOf,
} from "./mentions.js";

/**
 * The text model under the composer's pills.
 *
 * Worth testing on its own because the pills are only a rendering of it: if
 * this disagrees about where a mention starts, the pill draws over the wrong
 * characters, and if it disagrees about which ids are in a message, the wrong
 * agent is asked to speak.
 */

const specs = mentionSpecs({
  agents: [
    { id: "a1", name: "Nova" },
    { id: "a2", name: "Nova Reyes" },
    { id: "a3", name: "Iris" },
  ],
  skills: [{ id: "s1", name: "Brand Guidelines" }, { id: "s2", name: "Review" }],
});

describe("turning a name into a token", () => {
  it("lowercases and joins the words", () => {
    expect(slugOf("Nova Reyes")).toBe("nova-reyes");
    expect(slugOf("  Brand   Guidelines  ")).toBe("brand-guidelines");
  });

  it("drops punctuation rather than carrying it into the token", () => {
    expect(slugOf("Q&A / Support")).toBe("q-a-support");
  });

  it("has nothing to say about a name that is all punctuation", () => {
    expect(slugOf("!!!")).toBe("");
    expect(mentionSpecs({ agents: [{ id: "x", name: "!!!" }] })).toEqual([]);
  });

  it("lets the caller supply the face, and its answer wins", () => {
    const [nova] = mentionSpecs({
      agents: [{ id: "a", name: "Nova", avatarUrl: "data:x", avatarColor: "#123" }],
      face: () => ({ iconSrc: null, iconHtml: "<svg/>" }),
    });
    expect(nova).toMatchObject({ iconSrc: null, iconHtml: "<svg/>", tint: "#123", initials: "N" });
  });

  it("keeps the first of two names that make the same token", () => {
    const both = mentionSpecs({ agents: [{ id: "a", name: "Nova" }, { id: "b", name: "nova" }] });
    expect(both).toHaveLength(1);
    expect(both[0].id).toBe("a");
  });
});

describe("finding the mentions in a message", () => {
  it("finds one of each kind", () => {
    const found = findSpecRanges("@nova please run /review", specs);
    expect(found.map((r) => r.spec.raw)).toEqual(["@nova", "/review"]);
  });

  it("prefers the longer token where two would match", () => {
    const found = findSpecRanges("ask @nova-reyes about it", specs);
    expect(found).toHaveLength(1);
    expect(found[0].spec.id).toBe("a2");
  });

  it("does not match inside a word", () => {
    expect(findSpecRanges("mail me at nick@nova.dev", specs)).toEqual([]);
    expect(findSpecRanges("@novabuild is a product", specs)).toEqual([]);
  });

  it("does not read a path as a skill", () => {
    expect(findSpecRanges("open src/review/index.js", specs)).toEqual([]);
  });

  it("allows punctuation straight after a mention", () => {
    expect(findSpecRanges("thanks @iris!", specs).map((r) => r.spec.id)).toEqual(["a3"]);
  });

  it("reports the same mention twice when it is written twice", () => {
    expect(findSpecRanges("@iris and @iris", specs)).toHaveLength(2);
  });
});

describe("what the caret is in the middle of", () => {
  it("reports a token being typed", () => {
    expect(activeQuery("tell @no", 8, specs)).toMatchObject({ kind: "agent", query: "no", start: 5, end: 8 });
  });

  it("reports a bare sigil, so the palette opens before anything is typed", () => {
    expect(activeQuery("/", 1, specs)).toMatchObject({ kind: "skill", query: "" });
  });

  it("says nothing once the query has a space in it", () => {
    expect(activeQuery("@nova and", 9, specs)).toBeNull();
  });

  it("says nothing for a sigil in the middle of a word", () => {
    expect(activeQuery("nick@no", 7, specs)).toBeNull();
    expect(activeQuery("src/rev", 7, specs)).toBeNull();
  });

  it("says nothing when the caret is inside a finished pill", () => {
    // Backspacing the space after an accepted mention must not reopen it.
    expect(caretInMention("@nova", specs, 5)).toBe(true);
    expect(activeQuery("@nova", 5, specs)).toBeNull();
  });

  it("says nothing in plain prose", () => {
    expect(activeQuery("just a sentence", 15, specs)).toBeNull();
  });
});

describe("offering the choices", () => {
  it("puts prefix matches first and the shorter name above the longer", () => {
    const hits = matchMentions(specs, { kind: "agent", query: "no" });
    expect(hits.map((s) => s.label)).toEqual(["Nova", "Nova Reyes"]);
  });

  it("still offers a match in the middle of a name", () => {
    expect(matchMentions(specs, { kind: "agent", query: "rey" }).map((s) => s.id)).toEqual(["a2"]);
  });

  it("offers everything of the kind when nothing has been typed", () => {
    expect(matchMentions(specs, { kind: "skill", query: "" })).toHaveLength(2);
  });

  it("never crosses the kinds", () => {
    expect(matchMentions(specs, { kind: "skill", query: "nova" })).toEqual([]);
  });
});

describe("accepting one", () => {
  it("replaces what was typed and leaves the caret past a space", () => {
    const range = activeQuery("tell @no", 8, specs);
    const spec = matchMentions(specs, range)[0];
    expect(applyMention("tell @no", range, spec)).toEqual({ text: "tell @nova ", caret: 11 });
  });

  it("does not double the space when there is one already", () => {
    const range = activeQuery("@no rest", 3, specs);
    const spec = matchMentions(specs, range)[0];
    expect(applyMention("@no rest", range, spec).text).toBe("@nova rest");
  });
});

describe("what a message attaches", () => {
  it("lists the ids in the order they were written, without repeats", () => {
    expect(mentionedIds("@iris and @nova, then @iris again, with /review", specs)).toEqual({
      agent: ["a3", "a1"],
      skill: ["s2"],
      command: [],
    });
  });

  it("finds nothing in a message with no mentions", () => {
    expect(mentionedIds("carry on", specs)).toEqual({ agent: [], skill: [], command: [] });
  });
});

describe("commands share the slash with skills", () => {
  const withCommands = mentionSpecs({
    skills: [{ id: "s2", name: "Review" }, { id: "s3", name: "Copy Edit" }],
    commands: [{ name: "compact", summary: "Summarise the conversation" }],
  });

  it("offers both under one query", () => {
    const range = activeQuery("/", 1, withCommands);
    const names = matchMentions(withCommands, range).map((spec) => spec.raw);
    expect(names).toContain("/compact");
    expect(names).toContain("/review");
  });

  it("puts the command first when both would do", () => {
    const range = activeQuery("/c", 2, withCommands);
    expect(matchMentions(withCommands, range)[0].kind).toBe("command");
  });
});
