import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * The unfinished message.
 *
 * Run against a stand-in for localStorage, because the point of the feature is
 * that the text outlives the component - and the only way to prove that is to
 * throw the module away and read it back, which is what a reload does.
 */

function fakeStorage() {
  const map = new Map();
  return {
    get length() {
      return map.size;
    },
    key: (i) => [...map.keys()][i] ?? null,
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, String(v)),
    removeItem: (k) => map.delete(k),
    clear: () => map.clear(),
    _map: map,
  };
}

let drafts;

beforeEach(async () => {
  vi.useFakeTimers();
  globalThis.localStorage = fakeStorage();
  vi.resetModules();
  drafts = await import("./drafts.js");
});

afterEach(() => {
  vi.useRealTimers();
  delete globalThis.localStorage;
});

/** Past the debounce, which is where the write actually happens. */
const settle = () => vi.advanceTimersByTime(500);

describe("a draft that outlives the composer", () => {
  it("comes back after the module is thrown away and reloaded", async () => {
    drafts.saveDraft("t1", "half a thought");
    settle();

    // A reload is a fresh module against the same storage.
    vi.resetModules();
    const reloaded = await import("./drafts.js");
    expect(reloaded.loadDraft("t1")).toBe("half a thought");
  });

  it("keeps one draft per conversation", () => {
    drafts.saveDraft("t1", "for the first");
    drafts.saveDraft("t2", "for the second");
    settle();
    expect(drafts.loadDraft("t1")).toBe("for the first");
    expect(drafts.loadDraft("t2")).toBe("for the second");
  });

  it("reads back before the debounce has fired", () => {
    // Switching screens is faster than 250ms, and the text has to be there.
    drafts.saveDraft("t1", "typed just now");
    expect(drafts.loadDraft("t1")).toBe("typed just now");
  });

  it("forgets a draft that was sent", () => {
    drafts.saveDraft("t1", "about to send");
    settle();
    drafts.clearDraft("t1");
    settle();
    expect(drafts.loadDraft("t1")).toBe("");
    expect(drafts.draftThreadIds()).not.toContain("t1");
  });

  it("does not store whitespace as a draft", () => {
    drafts.saveDraft("t1", "   \n  ");
    settle();
    expect(drafts.draftThreadIds()).toEqual([]);
  });

  it("survives a storage that refuses to answer", () => {
    // A private window, a full quota, storage disabled. Losing a draft is bad;
    // taking the composer down with it is worse.
    globalThis.localStorage = {
      getItem: () => {
        throw new Error("nope");
      },
      setItem: () => {
        throw new Error("nope");
      },
      removeItem: () => {},
      key: () => null,
      length: 0,
    };
    expect(() => drafts.saveDraft("t1", "text")).not.toThrow();
    settle();
    expect(drafts.loadDraft("t1")).toBe("text");
  });

  it("drops drafts that have gone stale", () => {
    const { prune, MAX_AGE_MS } = drafts._internals;
    const now = Date.now();
    const kept = prune(
      {
        fresh: { text: "recent", at: now - 1000 },
        ancient: { text: "from last year", at: now - MAX_AGE_MS - 1 },
      },
      now
    );
    expect(Object.keys(kept)).toEqual(["fresh"]);
  });

  it("keeps only the newest when there are too many", () => {
    const { prune, MAX_DRAFTS } = drafts._internals;
    const now = Date.now();
    const many = {};
    for (let i = 0; i < MAX_DRAFTS + 25; i += 1) many[`t${i}`] = { text: `d${i}`, at: now - i };
    const kept = prune(many, now);
    expect(Object.keys(kept)).toHaveLength(MAX_DRAFTS);
    // Newest first, so the one written a moment ago is never the one dropped.
    expect(kept.t0).toBeTruthy();
  });
});
