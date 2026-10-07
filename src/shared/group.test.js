import { describe, expect, it } from "vitest";
import { mentionedAgents, slugOf } from "./group.js";

/**
 * Reading `@handles` out of what an agent wrote.
 *
 * This is how agents ask each other things without a tool call, so it has to
 * agree with the composer about what a handle is and refuse the things that
 * merely look like one.
 */

const agents = [
  { id: "a1", name: "Nova" },
  { id: "a2", name: "Nova Reyes" },
  { id: "a3", name: "Folder Organizer" },
];

describe("handles in a reply", () => {
  it("finds the agents named, in order, once each", () => {
    expect(
      mentionedAgents("@folder-organizer what do you think? @nova too. @folder-organizer", agents)
    ).toEqual(["a3", "a1"]);
  });

  it("reads the whole hyphenated word as the handle", () => {
    expect(mentionedAgents("ask @nova-reyes", agents)).toEqual(["a2"]);
    expect(mentionedAgents("ask @nova-something", agents)).toEqual([]);
  });

  it("is not fooled by case or trailing punctuation", () => {
    expect(mentionedAgents("Over to you, @Folder-Organizer!", agents)).toEqual(["a3"]);
  });

  it("does not read an email address or a mid-word @ as a mention", () => {
    expect(mentionedAgents("mail nick@nova.dev", agents)).toEqual([]);
  });

  it("has nothing to say about plain prose", () => {
    expect(mentionedAgents("Folder Organizer, what do you think?", agents)).toEqual([]);
    expect(mentionedAgents("", agents)).toEqual([]);
    expect(mentionedAgents(undefined, agents)).toEqual([]);
  });

  it("makes the same handle the composer does", () => {
    expect(slugOf("Folder Organizer")).toBe("folder-organizer");
    expect(slugOf("  Q&A / Support ")).toBe("q-a-support");
  });
});
