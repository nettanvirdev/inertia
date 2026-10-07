import { describe, expect, it } from "vitest";
import {
  cutScript,
  cutScriptParts,
  isPass,
  stripSelfLabel,
  stripSelfLabelParts,
} from "./self-label.js";

/**
 * The name an agent writes in front of its own reply.
 *
 * The transcript the model reads is labelled "Name: ..." on every reply, and
 * the weaker models copy it. On screen the name is already on the bubble; in
 * the next transcript it would be "Name: Name: ...". This is the one function
 * both places use to take it off.
 */
describe("a reply that opens with its own name", () => {
  it("loses the name and the separator", () => {
    expect(stripSelfLabel("Folder Organizer: Hi. Which folder?", "Folder Organizer")).toBe(
      "Hi. Which folder?"
    );
  });

  it("copes with the bold and the dash the models also produce", () => {
    expect(stripSelfLabel("**Nova:** on it", "Nova")).toBe("on it");
    expect(stripSelfLabel("Nova - on it", "Nova")).toBe("on it");
    expect(stripSelfLabel("  nova: on it", "Nova")).toBe("on it");
  });

  it("leaves a name in the middle of a sentence alone", () => {
    expect(stripSelfLabel("Ask Nova: she knows.", "Nova")).toBe("Ask Nova: she knows.");
  });

  it("leaves a different agent's name alone", () => {
    expect(stripSelfLabel("Iris: over to you", "Nova")).toBe("Iris: over to you");
  });

  it("is safe with names that look like regex", () => {
    expect(stripSelfLabel("Q&A (Support): hello", "Q&A (Support)")).toBe("hello");
  });

  it("does nothing without a name or text", () => {
    expect(stripSelfLabel("Nova: hi", null)).toBe("Nova: hi");
    expect(stripSelfLabel("", "Nova")).toBe("");
    expect(stripSelfLabel(undefined, "Nova")).toBe("");
  });
});

describe("the same, on a reply's parts", () => {
  it("strips only the first text part", () => {
    const parts = [
      { type: "reasoning", text: "Nova: thinking" },
      { type: "text", text: "Nova: first" },
      { type: "text", text: "Nova: second" },
    ];
    const out = stripSelfLabelParts(parts, "Nova");
    expect(out[0].text).toBe("Nova: thinking");
    expect(out[1].text).toBe("first");
    expect(out[2].text).toBe("Nova: second");
  });

  it("returns the same array when there is nothing to strip", () => {
    const parts = [{ type: "text", text: "hello" }];
    expect(stripSelfLabelParts(parts, "Nova")).toBe(parts);
    expect(stripSelfLabelParts(parts, null)).toBe(parts);
    expect(stripSelfLabelParts(undefined, "Nova")).toBeUndefined();
  });
});

describe("a reply that turns into a script for the others", () => {
  const others = ["Folder Organizer", "Iris"];

  it("is cut where the first colleague's line begins", () => {
    const text =
      "I'll start: purpose over type.\nFolder Organizer: I disagree.\nInertia Dev: fair.";
    expect(cutScript(text, others)).toBe("I'll start: purpose over type.");
  });

  it("is cut when the script begins on the first line", () => {
    expect(cutScript("**Iris:** hello there", others)).toBe("");
  });

  it("is left alone when a colleague is named mid-sentence", () => {
    const text = "I agree with Iris: purpose first. Ask Folder Organizer: he sorts.";
    expect(cutScript(text, others)).toBe(text);
  });

  it("is left alone with no colleagues to write for", () => {
    expect(cutScript("Iris: hi", [])).toBe("Iris: hi");
    expect(cutScript(undefined, others)).toBe("");
  });

  it("drops the parts after the cut, tool calls included", () => {
    const parts = [
      { type: "text", text: "Mine.\nIris: hers" },
      { type: "tool", callId: "c1", name: "read" },
      { type: "text", text: "more script" },
    ];
    expect(cutScriptParts(parts, others)).toEqual([{ type: "text", text: "Mine." }]);
  });

  it("drops a part that was nothing but script", () => {
    expect(cutScriptParts([{ type: "text", text: "Iris: all hers" }], others)).toEqual([]);
  });

  it("returns the same array when nothing was scripted", () => {
    const parts = [{ type: "text", text: "just me" }];
    expect(cutScriptParts(parts, others)).toBe(parts);
  });
});

describe("an agent passing its turn", () => {
  it("is the word on its own, however it is dressed", () => {
    expect(isPass("pass")).toBe(true);
    expect(isPass("  Pass. ")).toBe(true);
    expect(isPass("**pass**")).toBe(true);
  });

  it("is not a sentence that happens to contain it", () => {
    expect(isPass("I'll pass on that one")).toBe(false);
    expect(isPass("")).toBe(false);
  });
});
