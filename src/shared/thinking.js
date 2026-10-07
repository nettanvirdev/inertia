/**
 * How much a model may think, in the terms that model understands.
 *
 * The agent editor used to offer one control - a token budget in four named
 * steps - to every model. That is Anthropic's dial, and it is the only one
 * the Anthropic transport reads; on any other model the choice was sent
 * nowhere and did nothing. "Careful" on Minimax was a label over a field the
 * request never carried.
 *
 * opencode handles this by deriving a model's *variants* from its id and its
 * transport: Claude gets a budget, OpenAI's reasoning models get an effort
 * level, and models that decide for themselves get no control at all rather
 * than a fake one. This is the same idea, in the shape this app can afford: it
 * has no model catalogue to consult, so the id is what there is to go on.
 *
 * Both halves read it. The editor asks what to show; the transports ask what
 * to send. One table, so the two cannot disagree about which model gets what.
 */

/** Anthropic's dial: a budget of thinking tokens, or none. */
export const BUDGETS = [
  { value: "0", label: "Off", description: "Answers straight away" },
  { value: "2048", label: "Brief", description: "A moment to plan" },
  { value: "8192", label: "Careful", description: "Works problems through" },
  { value: "24576", label: "Deep", description: "Slow and expensive" },
];

/**
 * OpenAI's dial: `reasoning_effort`. "Default" leaves the field out, which is
 * what every reasoning model does when nobody says, and is the one value that
 * cannot be rejected by an endpoint that supports fewer tiers than the list.
 */
export const EFFORTS = [
  { value: "default", label: "Default", description: "The model's own setting" },
  { value: "low", label: "Low", description: "A moment to plan" },
  { value: "medium", label: "Medium", description: "Works problems through" },
  { value: "high", label: "High", description: "Slow and expensive" },
];

/** Shown when a model has no dial, so the field says why rather than lying. */
export const NONE = [
  { value: "none", label: "Not adjustable", description: "This model decides for itself" },
];

/** Families whose OpenAI-compatible endpoints take `reasoning_effort`. */
const EFFORT_FAMILIES = [
  /(^|\/)gpt-5([.-]|$)/,
  /(^|\/)o[134](-|$)/,
  /gpt-oss/,
  /codex/,
  /grok-3-mini/,
  /grok-4/,
];

/**
 * Which control a model gets.
 *
 * `protocol` is the provider's wire protocol, "anthropic" or "openai". Claude
 * through an OpenAI-compatible gateway still thinks in budgets on Anthropic's
 * side of the gateway, but the gateway takes `reasoning_effort` for it - so
 * the protocol decides, and the id only refines within it.
 *
 * Returns `{ kind, options }` where `kind` is "budget", "effort" or "none",
 * and `options` is what to put in a picker.
 */
export function thinkingControl(modelId, protocol = "openai") {
  const id = String(modelId ?? "").toLowerCase();
  if (protocol === "anthropic") return { kind: "budget", options: BUDGETS };
  if (id.includes("claude")) return { kind: "effort", options: EFFORTS };
  if (EFFORT_FAMILIES.some((family) => family.test(id)))
    return { kind: "effort", options: EFFORTS };
  return { kind: "none", options: NONE };
}

/**
 * What to send for an agent's stored choice, given the model it is about to
 * talk to. An agent whose model changed keeps its old fields; only the one
 * that applies to the new model is honoured, and the other is ignored rather
 * than sent to a model that would reject it.
 */
export function thinkingFor(agent, modelId, protocol) {
  const control = thinkingControl(modelId, protocol);
  if (control.kind === "budget") {
    return { thinkingBudget: Number(agent?.thinkingBudget) || 0, reasoningEffort: null };
  }
  if (control.kind === "effort") {
    const effort = String(agent?.reasoningEffort ?? "");
    const known = EFFORTS.some((option) => option.value === effort && effort !== "default");
    return { thinkingBudget: 0, reasoningEffort: known ? effort : null };
  }
  return { thinkingBudget: 0, reasoningEffort: null };
}
