import { describe, expect, it } from "vitest";
import {
  addUsage,
  costOf,
  formatCost,
  formatTokens,
  isLocalEndpoint,
  normalizeUsage,
  priceFor,
} from "./usage.js";

describe("normalizeUsage", () => {
  it("reads the shape every OpenAI-compatible endpoint sends", () => {
    expect(normalizeUsage({ prompt_tokens: 10, completion_tokens: 4 })).toEqual({
      input: 10,
      output: 4,
      cached: 0,
      cacheWrite: 0,
      reasoning: 0,
      requests: 1,
    });
  });

  it("finds the cached and reasoning counts wherever a gateway put them", () => {
    const usage = normalizeUsage({
      prompt_tokens: 100,
      completion_tokens: 50,
      prompt_tokens_details: { cached_tokens: 80 },
      completion_tokens_details: { reasoning_tokens: 30 },
    });
    expect(usage.cached).toBe(80);
    expect(usage.reasoning).toBe(30);
  });

  it("answers nothing for a provider that reported nothing", () => {
    // A total with no breakdown cannot be priced, and pretending otherwise
    // would put a made-up cost on the screen.
    expect(normalizeUsage({ total_tokens: 42 })).toBeNull();
    expect(normalizeUsage(null)).toBeNull();
    expect(normalizeUsage("120 tokens")).toBeNull();
  });
});

describe("addUsage", () => {
  it("sums every field and counts the requests", () => {
    const a = { input: 50, output: 10, cached: 0, reasoning: 0, requests: 1 };
    const b = { input: 100, output: 20, cached: 40, reasoning: 5, requests: 1 };
    expect(addUsage(a, b)).toEqual({
      input: 150,
      output: 30,
      cached: 40,
      cacheWrite: 0,
      reasoning: 5,
      requests: 2,
    });
  });

  it("copes with either side being absent, which is the common case", () => {
    const b = { input: 1, output: 2, cached: 0, reasoning: 0, requests: 1 };
    expect(addUsage(null, b)).toEqual(b);
    expect(addUsage(b, null)).toEqual(b);
    expect(addUsage(null, null)).toBeNull();
  });
});

describe("priceFor", () => {
  it("finds a shipped price by model family", () => {
    expect(priceFor("gpt-4o-mini")).toMatchObject({ input: 0.15, output: 0.6, source: "shipped" });
  });

  it("prefers the more specific entry", () => {
    // `gpt-4.1-mini` must not be read as `gpt-4.1`.
    expect(priceFor("gpt-4.1-mini").input).toBe(0.4);
    expect(priceFor("gpt-4.1").input).toBe(2);
  });

  it("lets the workspace beat the shipped number", () => {
    const price = priceFor("gpt-4o", { "gpt-4o": { input: 1, output: 3 } });
    expect(price).toEqual({ input: 1, output: 3, source: "workspace" });
  });

  it("says nothing rather than guessing at a model it does not know", () => {
    expect(priceFor("some-local-7b")).toBeNull();
    expect(priceFor("")).toBeNull();
  });
});

describe("costOf", () => {
  it("bills input and output at their own rates", () => {
    const usage = { input: 1_000_000, output: 1_000_000, cached: 0 };
    expect(costOf(usage, { input: 2, output: 8 })).toBe(10);
  });

  it("bills the cached share of the input at a tenth, without counting it twice", () => {
    // Cached tokens are already inside `input`, so the discount is a
    // substitution rather than an extra term.
    const usage = { input: 1_000_000, output: 0, cached: 1_000_000 };
    expect(costOf(usage, { input: 10, output: 10 })).toBe(1);
  });

  it("is null, never zero, when nobody has priced the model", () => {
    expect(costOf({ input: 5000, output: 500 }, null)).toBeNull();
    expect(costOf(null, { input: 1, output: 1 })).toBeNull();
  });
});

describe("formatting", () => {
  it("never rounds a real cost down to free", () => {
    expect(formatCost(0.00001)).toBe("<$0.0001");
    expect(formatCost(0)).toBe("$0");
    expect(formatCost(0.0143)).toBe("$0.0143");
    expect(formatCost(1.239)).toBe("$1.24");
    expect(formatCost(null)).toBeNull();
  });

  it("shortens a token count without losing the scale", () => {
    expect(formatTokens(1204)).toBe("1,204");
    expect(formatTokens(18_240)).toBe("18.2k");
    expect(formatTokens(1_400_000)).toBe("1.40M");
  });
});

describe("isLocalEndpoint", () => {
  it("knows a machine that charges nothing from one that does", () => {
    expect(isLocalEndpoint("http://localhost:11434/v1")).toBe(true);
    expect(isLocalEndpoint("http://127.0.0.1:8080")).toBe(true);
    expect(isLocalEndpoint("https://api.openai.com/v1")).toBe(false);
  });
});
