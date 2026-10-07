import { afterEach, describe, expect, it, vi } from "vitest";

import {
  DEFAULT_MAX_AGE_MS,
  _reset,
  invalidate,
  isStale,
  load,
  peek,
  subscribe,
  write,
} from "./cache.js";

/**
 * The cache without React, which is all of the behaviour that can be wrong:
 * what is remembered, how old it is allowed to get, and whether two readers
 * asking at once ask the backend twice.
 */

afterEach(() => {
  _reset();
  vi.useRealTimers();
});

describe("remembering an answer", () => {
  it("hands back what was written, with nothing before that", () => {
    expect(peek("providers")).toBeUndefined();
    write("providers", [{ id: "docker" }]);
    expect(peek("providers").value).toEqual([{ id: "docker" }]);
  });

  it("tells every reader of that key, and only that key", () => {
    const seen = [];
    const other = [];
    subscribe("a", (value) => seen.push(value));
    subscribe("a", (value) => seen.push(value));
    subscribe("b", (value) => other.push(value));
    write("a", 1);
    expect(seen).toEqual([1, 1]);
    expect(other).toEqual([]);
  });

  it("keeps telling the others when one reader throws", () => {
    // A listener that throws is a component that unmounted mid-notify.
    const seen = [];
    subscribe("a", () => {
      throw new Error("gone");
    });
    subscribe("a", (value) => seen.push(value));
    expect(() => write("a", 7)).not.toThrow();
    expect(seen).toEqual([7]);
  });

  it("stops telling a reader that unsubscribed", () => {
    const seen = [];
    const off = subscribe("a", (value) => seen.push(value));
    write("a", 1);
    off();
    write("a", 2);
    expect(seen).toEqual([1]);
  });
});

describe("how old is too old", () => {
  it("is stale when it has never been fetched", () => {
    expect(isStale("nothing")).toBe(true);
  });

  it("is fresh immediately after a write and stale after the window", () => {
    vi.useFakeTimers();
    write("a", 1);
    expect(isStale("a")).toBe(false);
    vi.advanceTimersByTime(DEFAULT_MAX_AGE_MS - 1);
    expect(isStale("a")).toBe(false);
    vi.advanceTimersByTime(2);
    expect(isStale("a")).toBe(true);
  });

  it("takes a window of its own", () => {
    vi.useFakeTimers();
    write("a", 1);
    vi.advanceTimersByTime(100);
    expect(isStale("a", 50)).toBe(true);
    expect(isStale("a", 1000)).toBe(false);
  });
});

describe("fetching", () => {
  it("remembers what the loader returned", async () => {
    await load("a", async () => "fetched");
    expect(peek("a").value).toBe("fetched");
  });

  it("asks once when two readers ask at the same moment", async () => {
    let calls = 0;
    const loader = () =>
      new Promise((resolve) => {
        calls += 1;
        setTimeout(() => resolve(calls), 0);
      });
    const [one, two] = await Promise.all([load("a", loader), load("a", loader)]);
    expect(calls).toBe(1);
    expect(one).toBe(1);
    expect(two).toBe(1);
  });

  it("asks again once the first request has settled", async () => {
    let calls = 0;
    const loader = async () => ++calls;
    await load("a", loader);
    await load("a", loader);
    expect(calls).toBe(2);
  });

  it("leaves nothing behind when the loader fails, so the next reader retries", async () => {
    await expect(load("a", async () => { throw new Error("no bridge"); })).rejects.toThrow("no bridge");
    expect(peek("a")).toBeUndefined();
    await load("a", async () => "second time");
    expect(peek("a").value).toBe("second time");
  });

  it("does not keep a stale value from a failed refresh", async () => {
    await load("a", async () => "good");
    await expect(load("a", async () => { throw new Error("gone"); })).rejects.toThrow();
    // The last good answer is still what a screen shows.
    expect(peek("a").value).toBe("good");
  });
});

describe("forgetting", () => {
  it("drops one key, or all of them", async () => {
    write("a", 1);
    write("b", 2);
    invalidate("a");
    expect(peek("a")).toBeUndefined();
    expect(peek("b").value).toBe(2);
    invalidate();
    expect(peek("b")).toBeUndefined();
  });
});
