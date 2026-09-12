/**
 * The permission engine.
 *
 * One question, asked everywhere: may this agent do this thing, and if so does
 * a human have to say yes first. Three answers - `allow`, `ask`, `deny` - and a
 * ruleset that maps a tool (optionally a specific argument to it) onto one.
 *
 * The shape is opencode's, because it is the right one: a rule is a pattern
 * plus an action, patterns are ordered, and the most specific match wins. What
 * that buys is the thing a coarse on/off switch cannot express - "the terminal
 * is fine, but `git push` asks first, and `rm -rf` never" is three rules over
 * one tool, and it is the rule people actually want.
 *
 * This module is deliberately pure and dependency-free: the renderer imports it
 * to render and edit rules, and the main process will import it to enforce them.
 * A second implementation on the other side of the bridge would be a second set
 * of answers to a question that must only ever have one.
 */

export const ACTIONS = ["allow", "ask", "deny"];

export const DEFAULT_ACTION = "ask";

/** A rule with no explicit target covers everything the tool can do. */
export const ANY = "*";

/* -- patterns ---------------------------------------------------------- */

/**
 * Glob matching, restricted on purpose to `*` and `?`.
 *
 * These patterns are written by hand into a permissions screen, and a rule the
 * author cannot predict the meaning of is worse than no rule. Full regex would
 * make `git commit -m "fix(auth)"` an accident waiting to happen.
 */
export function matches(pattern, value) {
  if (pattern === ANY) return true;
  const source = String(pattern ?? "");
  const target = String(value ?? "");
  if (!source.includes("*") && !source.includes("?")) return source === target;

  const escaped = source.replace(/[.+^${}()|[\]\\]/g, "\\$&");
  const regex = new RegExp(`^${escaped.replace(/\*/g, ".*").replace(/\?/g, ".")}$`, "s");
  return regex.test(target);
}

/**
 * How specific a pattern is, so the winner between two matching rules is the
 * one that had more to say. Wildcards cost; literal characters earn.
 *
 * `*` is 0, `git *` beats it, `git push *` beats that, and an exact string
 * beats them all. Ordering by specificity rather than by position means a user
 * can add a rule anywhere in the list and get the meaning they expect, which is
 * not true of first-match-wins.
 */
export function specificity(pattern) {
  if (pattern === ANY) return 0;
  const source = String(pattern ?? "");
  const wildcards = (source.match(/[*?]/g) ?? []).length;
  return source.length - wildcards * 2 + (wildcards === 0 ? 1000 : 0);
}

/* -- rulesets ----------------------------------------------------------- */

/**
 * A ruleset is a flat, ordered list of `{ tool, pattern, action }`.
 *
 * Flat rather than nested-by-tool because it is edited as a list, diffed as a
 * list, and merged as a list. Nesting would only save typing in a file nobody
 * writes by hand.
 */
export function rule(tool, action, pattern = ANY) {
  return { tool, pattern, action };
}

/**
 * Shorthand to ruleset.
 *
 * Accepts what a config file naturally contains:
 *
 *   "deny"                              everything denied
 *   { shell: "ask" }                    one tool
 *   { shell: { "git push *": "ask" } }  one tool, per-argument
 */
export function fromConfig(config) {
  if (config == null) return [];
  if (typeof config === "string") return [rule(ANY, normalizeAction(config))];

  const rules = [];
  for (const [tool, value] of Object.entries(config)) {
    if (typeof value === "string") {
      rules.push(rule(tool, normalizeAction(value)));
      continue;
    }
    if (value && typeof value === "object") {
      for (const [pattern, action] of Object.entries(value)) {
        rules.push(rule(tool, normalizeAction(action), pattern));
      }
    }
  }
  return rules;
}

/** Config shape, for writing back out to a file a human will read. */
export function toConfig(rules) {
  const config = {};
  for (const entry of rules ?? []) {
    if (entry.pattern === ANY) {
      // A bare action stays a bare action unless a per-argument rule forces the
      // object form, so the common case reads as `shell: ask`.
      if (typeof config[entry.tool] === "object") config[entry.tool][ANY] = entry.action;
      else config[entry.tool] = entry.action;
      continue;
    }
    if (typeof config[entry.tool] !== "object") {
      const previous = config[entry.tool];
      config[entry.tool] = previous ? { [ANY]: previous } : {};
    }
    config[entry.tool][entry.pattern] = entry.action;
  }
  return config;
}

function normalizeAction(value) {
  return ACTIONS.includes(value) ? value : DEFAULT_ACTION;
}

/**
 * The flat form, for a file where nesting costs more than it is worth.
 *
 *   shell: allow
 *   shell/git push *: ask
 *
 * An agent is markdown with frontmatter, and one indented block is as deep as
 * that format goes on purpose - so the pattern joins the tool with a `/` rather
 * than opening a second level. The first `/` splits; everything after it is the
 * pattern, which keeps `shell/git push *` readable and unambiguous.
 */
export const FLAT_SEPARATOR = "/";

export function fromFlat(map) {
  const rules = [];
  for (const [key, action] of Object.entries(map ?? {})) {
    const cut = key.indexOf(FLAT_SEPARATOR);
    const tool = cut === -1 ? key : key.slice(0, cut);
    const pattern = cut === -1 ? ANY : key.slice(cut + 1);
    if (!tool) continue;
    rules.push(rule(tool, normalizeAction(action), pattern || ANY));
  }
  return rules;
}

export function toFlat(rules) {
  const map = {};
  for (const entry of rules ?? []) {
    const key =
      entry.pattern === ANY ? entry.tool : `${entry.tool}${FLAT_SEPARATOR}${entry.pattern}`;
    map[key] = entry.action;
  }
  return map;
}

/**
 * Whatever a permissions document actually holds, as rules.
 *
 * The workspace folder is the source of truth and it is meant to be hand-edited,
 * which means the ruleset can be an older app's shape, a typo, or something a
 * person wrote by hand at midnight. Every reader downstream of this spreads the
 * result and calls `evaluate` on it, so the one thing it must never be is a
 * value that is neither null nor iterable - a `?? []` guard does not catch an
 * object, and an object is exactly what a folder written before rules were
 * arrays contains.
 *
 * An object is read as the flat form. Names that no tool answers to survive as
 * rules that never match, which is the truthful outcome: a document naming a
 * permission this app no longer has grants and denies nothing, and the agent
 * falls back to the workspace ruleset and then to the defaults.
 */
export function asRules(value) {
  if (Array.isArray(value)) {
    const rules = [];
    for (const entry of value) {
      if (!entry || typeof entry !== "object") continue;
      if (typeof entry.tool !== "string" || !entry.tool) continue;
      const pattern = typeof entry.pattern === "string" && entry.pattern ? entry.pattern : ANY;
      rules.push(rule(entry.tool, normalizeAction(entry.action), pattern));
    }
    return rules;
  }
  if (value && typeof value === "object") return fromFlat(value);
  return [];
}

/** The same, for the `agentId -> ruleset` half of the document. */
export function asRuleMap(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const map = {};
  for (const [agentId, rules] of Object.entries(value)) map[agentId] = asRules(rules);
  return map;
}

/**
 * Later rulesets win where they overlap.
 *
 * The order is defaults, then workspace, then the agent's own - so an agent can
 * always tighten or loosen what the workspace said, and the workspace can
 * always move off a default, without any layer having to know the others exist.
 */
export function merge(...rulesets) {
  const byKey = new Map();
  for (const rules of rulesets) {
    for (const entry of rules ?? []) {
      // A NUL between the two halves, written as an escape.
      //
      // It is the separator because it is the one character that cannot appear
      // in a tool name or a pattern, so `shell` + `git *` can never collide with
      // some other pair that happens to concatenate the same way. It used to be
      // a raw NUL byte sitting in the source: correct, and invisible in every
      // editor and every diff, which is one careless save away from becoming a
      // silent behaviour change nobody can see.
      byKey.set(`${entry.tool}\u0000${entry.pattern}`, entry);
    }
  }
  return [...byKey.values()];
}

/* -- the question -------------------------------------------------------- */

/**
 * What happens if this agent calls this tool with this argument.
 *
 * `target` is the part of the call worth writing a rule about - the command for
 * a shell, the path for a write, the host for a fetch. Tools that have no such
 * part pass nothing and only ever match `*`.
 *
 * Returns the winning rule so a caller can say WHY, which matters: "denied by
 * your rule `shell: rm -rf *`" is actionable and "denied" is not.
 */
export function evaluate(rules, tool, target = ANY) {
  let best = null;
  let bestScore = -1;

  for (const entry of rules ?? []) {
    // The tool side is a pattern too. That is what lets one rule cover a group
    // of keys at once, which matters for the runtime sources: their tool names
    // are not knowable until the thing is connected, and a permission you
    // cannot write in advance is one the user ends up granting in a hurry.
    if (!matches(entry.tool, tool)) continue;
    if (!matches(entry.pattern, target)) continue;

    // The tool dominates: a rule that names this exact tool beats any rule that
    // only globbed its way here, however specific that rule's argument pattern
    // is. Otherwise `*` with a long pattern could outrank a deliberate choice.
    const score = specificity(entry.tool) * 100000 + specificity(entry.pattern);
    if (score > bestScore) {
      bestScore = score;
      best = entry;
    }
  }

  return {
    action: best?.action ?? DEFAULT_ACTION,
    rule: best,
    // True when nothing in the ruleset had an opinion, so a UI can say "not
    // configured" rather than implying someone chose to be asked.
    implicit: !best,
  };
}

/** The common case: is this outright forbidden. */
export function isDenied(rules, tool, target) {
  return evaluate(rules, tool, target).action === "deny";
}

/**
 * Which tools the model is even told about.
 *
 * A tool denied outright - denied for every possible argument - is not offered
 * at all, because a model that can see a tool will eventually try it, and a
 * refusal it cannot avoid is a wasted turn and a confused transcript. A tool
 * denied only for some arguments stays visible: the model can still use it
 * correctly, and the narrow deny does its job at call time.
 */
export function visibleTools(tools, rules) {
  return (tools ?? []).filter((tool) => {
    const id = typeof tool === "string" ? tool : tool.id;
    const verdict = evaluate(rules, id, ANY);
    return !(verdict.action === "deny" && (verdict.rule?.pattern ?? ANY) === ANY);
  });
}

/** Every tool that will stop and ask, for the "what will this agent check with
 *  me about" summary an agent's page should be able to show. */
export function toolsThatAsk(tools, rules) {
  return (tools ?? []).filter((tool) => {
    const id = typeof tool === "string" ? tool : tool.id;
    return evaluate(rules, id, ANY).action === "ask";
  });
}
