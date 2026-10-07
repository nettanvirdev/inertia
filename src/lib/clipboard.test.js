import { describe, it, expect, afterEach, vi } from "vitest";
import { copyText } from "./clipboard";

/**
 * The bug this file exists for: a copy control said "Copied to clipboard" and
 * the clipboard was untouched, because the write was never waited on and the
 * window's permission handler was refusing it.
 */
describe("copyText", () => {
  afterEach(() => vi.unstubAllGlobals());

  /** A document just real enough for the `execCommand` fallback. */
  function fakeDocument(execCopy) {
    const body = { appendChild: vi.fn() };
    return {
      body,
      activeElement: null,
      createElement: () => ({
        style: {},
        setAttribute: () => {},
        select: () => {},
        setSelectionRange: () => {},
        remove: () => {},
      }),
      execCommand: execCopy,
    };
  }

  it("says yes when the clipboard took the text", async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    expect(await copyText("hello")).toBe(true);
    expect(writeText).toHaveBeenCalledWith("hello");
  });

  it("falls back to the older way when the clipboard is refused", async () => {
    vi.stubGlobal("navigator", {
      clipboard: {
        writeText: async () => {
          throw new Error("denied");
        },
      },
    });
    const execCommand = vi.fn(() => true);
    vi.stubGlobal("document", fakeDocument(execCommand));
    expect(await copyText("hello")).toBe(true);
    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("says no when neither way works, so the toast can tell the truth", async () => {
    vi.stubGlobal("navigator", {
      clipboard: {
        writeText: async () => {
          throw new Error("denied");
        },
      },
    });
    vi.stubGlobal(
      "document",
      fakeDocument(() => false)
    );
    expect(await copyText("hello")).toBe(false);
  });

  it("says no to nothing at all rather than clearing what is there", async () => {
    const writeText = vi.fn(async () => {});
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    expect(await copyText("")).toBe(false);
    expect(await copyText(null)).toBe(false);
    expect(writeText).not.toHaveBeenCalled();
  });

  it("survives a window with no clipboard API at all", async () => {
    vi.stubGlobal("navigator", {});
    vi.stubGlobal(
      "document",
      fakeDocument(() => true)
    );
    expect(await copyText("hello")).toBe(true);
  });
});
