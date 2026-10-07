/**
 * Tokens, and what they cost.
 *
 * A turn is not one request. The model answers, asks for a tool, reads the
 * result and thinks again, and every one of those round trips is billed. The
 * loop used to keep `usage` in a variable it overwrote each step and emit only
 * the last one, so a turn that called five tools reported the cost of the
 * sixth request and threw the other five away - which makes the number worse
 * than useless, because it looks like an answer.
 *
 * So usage adds up. Every step contributes, the running total travels with the
 * turn, and the total is what gets stored and shown.
 *
 * On price: this app talks to whatever endpoint the user points it at, and no
 * endpoint reliably reports what it charged. A table is the only way, and a
 * table shipped in a binary is out of date the week after it ships. Two rules
 * follow from that, and they are the whole design:
 *
 * - The workspace can override any price, per model, in `settings/models.json`.
 *   The user's own numbers beat ours, always.
 * - A model with no price gets no cost. Not zero - null. Zero is a claim that
 *   the turn was free, and quietly under-reporting a bill is the one failure
 *   mode that would make someone regret trusting this screen.
 */

/** When the shipped numbers were compiled. Shown wherever a cost is, so the
 *  figure is read as an estimate with a date on it rather than as a receipt. */
export const PRICES_AS_OF = "2026-09";

/**
 * USD per million tokens, `[input, output]`, keyed by a pattern the model id is
 * matched against. Ordered: the first pattern that matches wins, so specific
 * entries come before the family they belong to.
 *
 * Deliberately short. It covers what the providers pane can reach today, and a
 * model it does not name is not a bug to be fixed by guessing - it is a row the
 * user can fill in for their own endpoint, which is the only way this stays
 * true for a local llama.cpp or a gateway with negotiated pricing.
 */
export const SHIPPED_PRICES = [
  // OpenAI
  ["gpt-4.1-nano", 0.1, 0.4],
  ["gpt-4.1-mini", 0.4, 1.6],
  ["gpt-4.1", 2, 8],
  ["gpt-4o-mini", 0.15, 0.6],
  ["gpt-4o", 2.5, 10],
  ["o4-mini", 1.1, 4.4],
  ["o3-mini", 1.1, 4.4],
  ["o3", 2, 8],
  // Anthropic
  ["claude-opus-4", 15, 75],
  ["claude-sonnet-4", 3, 15],
  ["claude-haiku-4", 1, 5],
  ["claude-3-5-haiku", 0.8, 4],
  // Google
  ["gemini-2.5-pro", 1.25, 10],
  ["gemini-2.5-flash", 0.3, 2.5],
  // Open weights, at the going rate on the fast hosts
  ["gpt-oss-120b", 0.15, 0.75],
  ["gpt-oss-20b", 0.1, 0.5],
  ["llama-3.3-70b", 0.59, 0.79],
  ["deepseek-chat", 0.27, 1.1],
  ["deepseek-reasoner", 0.55, 2.19],
  ["qwen3-32b", 0.29, 0.59],
];

/** A local model costs nothing to run, and saying "unknown" about it would be
 *  its own kind of wrong. Matched on the endpoint, not on the model name. */
const LOCAL_HOSTS = /^(https?:\/\/)?(localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])/i;

export const isLocalEndpoint = (baseUrl) => LOCAL_HOSTS.test(String(baseUrl ?? "").trim());

/* -- counting ------------------------------------------------------------ */

/**
 * A provider's usage object, in one shape.
 *
 * Every OpenAI-compatible endpoint reports `prompt_tokens` and
 * `completion_tokens`; some add a cached-read count in a nested object, and the
 * gateways disagree about where. Cached input is worth pulling out because it
 * is the difference between a long thread being affordable and not.
 */
export function normalizeUsage(raw) {
  if (!raw || typeof raw !== "object") return null;
  const num = (value) => (Number.isFinite(Number(value)) ? Number(value) : 0);

  const cached = num(
    raw.prompt_tokens_details?.cached_tokens ??
      raw.promptTokensDetails?.cachedTokens ??
      raw.cached_tokens ??
      raw.cache_read_input_tokens
  );
  // Tokens written into the cache for the first time. Only Anthropic reports
  // this, and it is billed above the base input rate rather than below it - so
  // folding it in with everything else would under-report exactly the request
  // that cost the most.
  const cacheWrite = num(raw.cache_creation_input_tokens ?? raw.cacheCreationInputTokens);

  const reported = num(raw.prompt_tokens ?? raw.promptTokens ?? raw.input_tokens);
  // The two APIs disagree about what the input count means, and the
  // disagreement is silent. An OpenAI-shaped `prompt_tokens` is everything that
  // went in, cached tokens included; Anthropic's `input_tokens` counts only
  // what was neither read from the cache nor written to it. Left alone, a
  // cached Anthropic turn would report a few hundred input tokens for a request
  // that carried fifty thousand, and every number downstream - the cost, the
  // context gauge, the running total - would be wrong in the reassuring
  // direction. One convention wins here: input is everything.
  const separate = raw.input_tokens != null && raw.prompt_tokens == null;
  const input = separate ? reported + cached + cacheWrite : reported;

  const output = num(raw.completion_tokens ?? raw.completionTokens ?? raw.output_tokens);
  const reasoning = num(raw.completion_tokens_details?.reasoning_tokens ?? raw.reasoning_tokens);

  if (!input && !output && !cached && !cacheWrite && !reasoning) return null;
  return { input, output, cached, cacheWrite, reasoning, requests: 1 };
}

/** Two usages, added. Either may be absent, which is the common case. */
export function addUsage(a, b) {
  if (!a) return b ? { ...b } : null;
  if (!b) return { ...a };
  return {
    input: (a.input ?? 0) + (b.input ?? 0),
    output: (a.output ?? 0) + (b.output ?? 0),
    cached: (a.cached ?? 0) + (b.cached ?? 0),
    cacheWrite: (a.cacheWrite ?? 0) + (b.cacheWrite ?? 0),
    reasoning: (a.reasoning ?? 0) + (b.reasoning ?? 0),
    requests: (a.requests ?? 0) + (b.requests ?? 0),
  };
}

export const totalTokens = (usage) => (usage?.input ?? 0) + (usage?.output ?? 0);

/* -- pricing ------------------------------------------------------------- */

/**
 * What one model costs per million tokens, or null if nobody has said.
 *
 * `overrides` is whatever the workspace holds: `{ "<model id>": { input, output } }`.
 * An exact id beats a pattern, because a user who typed a price for one model
 * meant that model and not its family.
 */
export function priceFor(modelId, overrides = {}) {
  const id = String(modelId ?? "").toLowerCase();
  if (!id) return null;

  const own = overrides?.[modelId] ?? overrides?.[id];
  if (own && Number.isFinite(Number(own.input)) && Number.isFinite(Number(own.output))) {
    return {
      input: Number(own.input),
      output: Number(own.output),
      // Optional, and left undefined rather than defaulted, so `costOf` can tell
      // "they said cached reads are free" from "they did not say".
      ...(Number.isFinite(Number(own.cachedInput)) ? { cachedInput: Number(own.cachedInput) } : {}),
      ...(Number.isFinite(Number(own.cacheWrite)) ? { cacheWrite: Number(own.cacheWrite) } : {}),
      source: "workspace",
    };
  }

  for (const [pattern, input, output] of SHIPPED_PRICES) {
    if (id.includes(pattern)) return { input, output, source: "shipped" };
  }
  return null;
}

/**
 * The multiplier on a token the first time it is cached.
 *
 * A read costs a tenth of the base rate everywhere that offers one, which is
 * the whole point. A write costs more than the base rate, because the provider
 * is storing something: a quarter more for the five-minute entry, double for
 * the hour-long one. This app asks for the hour, so that is the number used -
 * and the direction of the error, if a provider quietly gives us the cheaper
 * one, is that the screen over-states the bill rather than under-stating it.
 */
export const CACHE_WRITE_MULTIPLIER = 2;
export const CACHE_READ_MULTIPLIER = 0.1;

/**
 * What a turn cost, in dollars, or null when the model has no price.
 *
 * Input is one number covering three different prices. Cached reads are a tenth
 * of the rate, freshly cached writes are double it, and whatever was neither is
 * the rate itself - so the three shares are separated and priced apart rather
 * than added as extra terms, which would count the same tokens twice.
 */
export function costOf(usage, price) {
  if (!usage || !price) return null;
  const total = usage.input ?? 0;
  const cached = Math.min(usage.cached ?? 0, total);
  const written = Math.min(usage.cacheWrite ?? 0, total - cached);
  const fresh = Math.max(0, total - cached - written);

  // An explicit rate beats the multiplier every time. The multipliers are a good
  // guess at what most providers charge - a tenth to read, double to write - and
  // a guess is all they can be, because the ratio is not universal: it differs
  // between Anthropic and OpenAI, and a gateway with a negotiated contract can
  // charge anything it likes. So a user who knows their real cached rate can
  // type it, and only where they have not is the multiplier used.
  const cachedRate = rateOr(price.cachedInput, price.input * CACHE_READ_MULTIPLIER);
  const writeRate = rateOr(price.cacheWrite, price.input * CACHE_WRITE_MULTIPLIER);

  const dollars =
    (fresh * price.input +
      cached * cachedRate +
      written * writeRate +
      (usage.output ?? 0) * price.output) /
    1_000_000;
  return Math.round(dollars * 1e6) / 1e6;
}

/** An explicit rate if there is one - zero counts, and is not "unset". */
function rateOr(explicit, fallback) {
  const value = Number(explicit);
  return Number.isFinite(value) && value >= 0 ? value : fallback;
}

/**
 * How much of the input never had to be sent fresh, 0 to 1.
 *
 * Null rather than zero when nothing was cached either way, and the difference
 * matters. A provider that has never reported a cached read or a cache write is
 * one that does not do this, or does it invisibly - saying "0%" about it would
 * read as a failure. A provider that wrote a cache and read nothing back really
 * is at zero, which is worth seeing, because it is what a cache that has
 * stopped hitting looks like.
 */
export function cacheHitRate(usage) {
  const total = usage?.input ?? 0;
  if (!total) return null;
  if (!(usage.cached ?? 0) && !(usage.cacheWrite ?? 0)) return null;
  return Math.min(1, (usage.cached ?? 0) / total);
}

/** `$0.0143`, `$1.24`, or `<$0.0001` - never `$0.00` for a turn that cost
 *  something, because a rounded-away price reads as a free one. */
export function formatCost(dollars) {
  if (dollars == null || !Number.isFinite(dollars)) return null;
  if (dollars === 0) return "$0";
  if (dollars < 0.0001) return "<$0.0001";
  if (dollars < 1) return `$${dollars.toFixed(4)}`;
  return `$${dollars.toFixed(2)}`;
}

/** `1,204` / `18.2k` / `1.4M`, for a badge that has one line to say it in. */
export function formatTokens(count) {
  const n = Number(count) || 0;
  if (n < 10_000) return n.toLocaleString("en-US");
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}
