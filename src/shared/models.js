/**
 * What we know about a model without asking it.
 *
 * The app would rather not have this file. Its whole design is that a provider
 * is a URL the user typed, and a table of blessed model names is the opposite
 * of that. It exists because one number cannot be discovered and being wrong
 * about it is expensive.
 *
 * The number is the context window. `GET /models` reports it on some endpoints
 * and not others - Anthropic's does not report it at all - and when nobody has
 * said, the compactor assumes a deliberately small 32k. That assumption is safe
 * in the direction it was chosen for: guessing high means a request rejected
 * for length, which ends a turn. But applied to a model with a 200k window it
 * means summarising the conversation at 23k, over and over, throwing away
 * detail that would have fitted eight times over and paying for a summarising
 * call each time. The user sees an agent that forgets what it was doing, on a
 * model that had plenty of room.
 *
 * So: a short table, matched on a prefix of the model id, holding only what
 * cannot be asked for. It is not a list of supported models - anything the
 * user's endpoint offers still works - and a model that is not on it behaves
 * exactly as it did before this file existed.
 */

/**
 * `[pattern, context, maxOutput]`, first match wins, so a specific entry has to
 * come before the family it belongs to. Matched with `includes` against the
 * lowercased id, which is what makes a dated build like
 * `claude-sonnet-4-5-20250929` resolve without naming every release, and what
 * lets a gateway's `anthropic/claude-sonnet-4-5` resolve too.
 */
export const MODEL_LIMITS = [
  // Anthropic. Its own models endpoint reports no window at all, which is the
  // case this file was written for.
  ["claude-opus-4", 200000, 32000],
  ["claude-sonnet-4", 200000, 64000],
  ["claude-haiku-4", 200000, 32000],
  ["claude-3-7-sonnet", 200000, 64000],
  ["claude-3-5-sonnet", 200000, 8192],
  ["claude-3-5-haiku", 200000, 8192],
  ["claude-3-opus", 200000, 4096],
  ["claude-3-haiku", 200000, 4096],
  ["claude-", 200000, 8192],

  // OpenAI. Reported by the endpoint often enough that these are a backstop.
  ["gpt-4.1", 1047576, 32768],
  ["gpt-4o", 128000, 16384],
  ["o4-mini", 200000, 100000],
  ["o3-mini", 200000, 100000],
  ["o3", 200000, 100000],

  // Google.
  ["gemini-2.5-pro", 1048576, 65536],
  ["gemini-2.5-flash", 1048576, 65536],

  // Open weights, at the windows the fast hosts actually serve them with.
  ["gpt-oss-120b", 131072, 32768],
  ["gpt-oss-20b", 131072, 32768],
  ["llama-3.3-70b", 131072, 32768],
  ["deepseek-reasoner", 131072, 65536],
  ["deepseek-chat", 131072, 8192],
  ["qwen3", 131072, 32768],
];

function lookup(modelId) {
  const id = String(modelId ?? "").toLowerCase();
  if (!id) return null;
  for (const row of MODEL_LIMITS) {
    if (id.includes(row[0])) return row;
  }
  return null;
}

/** The context window this model is known to have, or null if nobody has said. */
export function knownContextWindow(modelId) {
  return lookup(modelId)?.[1] ?? null;
}

/** The most this model will write in one reply, or null. Required by some APIs
 *  and merely useful to the rest, which is why it is worth carrying. */
export function knownMaxOutput(modelId) {
  return lookup(modelId)?.[2] ?? null;
}
