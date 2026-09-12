import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  RETIRE_DELAY,
  isRetiring,
  keepPane,
  resetPaneRetire,
  retirePane,
} from "@/features/chat/pane-retire";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  resetPaneRetire();
  vi.useRealTimers();
});

describe("closing a view whose pane has gone", () => {
  it("closes it, a moment later", () => {
    const close = vi.fn();
    retirePane("chat:t1:web:1", close);
    expect(close).not.toHaveBeenCalled();
    vi.advanceTimersByTime(RETIRE_DELAY);
    expect(close).toHaveBeenCalledOnce();
  });

  it("does not close it when the same pane comes straight back", () => {
    // The case this module exists for. In development React unmounts and
    // remounts every component on purpose, and the old code read that as the
    // person closing the panel - destroying the page a tool was loading into
    // it.
    const close = vi.fn();
    retirePane("chat:t1:web:1", close);
    keepPane("chat:t1:web:1");
    vi.advanceTimersByTime(RETIRE_DELAY * 4);
    expect(close).not.toHaveBeenCalled();
  });

  it("keeps one pane's close from cancelling another's", () => {
    const first = vi.fn();
    const second = vi.fn();
    retirePane("chat:t1:web:1", first);
    retirePane("chat:t1:web:2", second);
    keepPane("chat:t1:web:1");
    vi.advanceTimersByTime(RETIRE_DELAY);
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledOnce();
  });

  it("replaces a pending close rather than stacking two", () => {
    const close = vi.fn();
    retirePane("chat:t1:web:1", close);
    retirePane("chat:t1:web:1", close);
    vi.advanceTimersByTime(RETIRE_DELAY * 2);
    expect(close).toHaveBeenCalledOnce();
  });

  it("says whether a close is still pending", () => {
    retirePane("chat:t1:web:1", () => {});
    expect(isRetiring("chat:t1:web:1")).toBe(true);
    vi.advanceTimersByTime(RETIRE_DELAY);
    expect(isRetiring("chat:t1:web:1")).toBe(false);
  });

  it("keeps a pane nobody retired, without complaining", () => {
    expect(keepPane("chat:t1:web:9")).toBe(false);
    expect(keepPane(null)).toBe(false);
  });

  it("ignores a retire with nothing to call or nothing to call it on", () => {
    expect(() => retirePane("", vi.fn())).not.toThrow();
    expect(() => retirePane("chat:t1:web:1", null)).not.toThrow();
    expect(isRetiring("chat:t1:web:1")).toBe(false);
  });

  it("survives a close that throws", () => {
    retirePane("chat:t1:web:1", () => {
      throw new Error("already gone");
    });
    expect(() => vi.advanceTimersByTime(RETIRE_DELAY)).not.toThrow();
  });
});
