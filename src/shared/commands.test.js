import { describe, expect, it } from "vitest";
import { COMMANDS, helpText, matchCommands, parseCommand, readToggle } from "./commands.js";

describe("parseCommand", () => {
  it("reads a bare command", () => {
    expect(parseCommand("/compact")).toMatchObject({ name: "compact", args: "" });
    expect(parseCommand("  /clear  ")).toMatchObject({ name: "clear", args: "" });
  });

  it("keeps everything after the name as the argument", () => {
    expect(parseCommand("/rename The Collatz room")).toMatchObject({
      name: "rename",
      args: "The Collatz room",
    });
  });

  it("does not care about case in the name", () => {
    expect(parseCommand("/COMPACT")?.name).toBe("compact");
  });

  it("leaves an ordinary message alone", () => {
    expect(parseCommand("what does /compact do?")).toBeNull();
    expect(parseCommand("look at /usr/bin/env")).toBeNull();
    expect(parseCommand("")).toBeNull();
    expect(parseCommand(null)).toBeNull();
  });

  it("does not claim a slash word it does not know", () => {
    expect(parseCommand("/deploy-notes for the release")).toBeNull();
    expect(parseCommand("/")).toBeNull();
  });
});

describe("matchCommands", () => {
  it("offers everything for an empty query", () => {
    expect(matchCommands("")).toHaveLength(COMMANDS.length);
  });

  it("puts a prefix match above a match in the middle", () => {
    const names = matchCommands("co").map((one) => one.name);
    expect(names.indexOf("compact")).toBeLessThan(names.indexOf("autocompact"));
  });

  it("finds nothing for a word no command contains", () => {
    expect(matchCommands("zzz")).toEqual([]);
  });
});

describe("readToggle", () => {
  it("reads the words people actually type", () => {
    expect(readToggle("on")).toBe(true);
    expect(readToggle("YES")).toBe(true);
    expect(readToggle("off")).toBe(false);
    expect(readToggle("disable")).toBe(false);
  });

  it("treats anything else as a question rather than an answer", () => {
    expect(readToggle("")).toBeNull();
    expect(readToggle("maybe")).toBeNull();
  });
});

describe("helpText", () => {
  it("names every command exactly once", () => {
    const text = helpText();
    for (const command of COMMANDS) expect(text).toContain(`/${command.name}`);
  });
});
