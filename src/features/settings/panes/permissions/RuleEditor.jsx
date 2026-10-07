import * as React from "react";
import { ANY, matches, specificity } from "@shared/permission";
import { Plus, X } from "@/components/icons";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Input } from "@/components/ui/input";
import { Segmented } from "@/components/ui/segmented";
import { cn } from "@/lib/utils";

/**
 * Editing a ruleset, one tool at a time.
 *
 * The engine in `@shared/permission` is a flat list of `{tool, pattern, action}`
 * where the most specific match wins. Everything in this file exists to keep
 * the screen honest about that: rules are added and removed anywhere in the
 * list because position carries no meaning, and the only structure the UI
 * imposes is "the `*` rule for this tool" versus "the narrower ones", which is
 * how a person thinks about it anyway.
 */

export const ACTION_OPTIONS = [
  { value: "allow", label: "Allow" },
  { value: "ask", label: "Ask" },
  { value: "deny", label: "Deny" },
];

export const ACTION_TONE = { allow: "success", ask: "warning", deny: "danger" };

/* -- ruleset helpers ----------------------------------------------------- */

/** The blanket rule for a tool, or null when the tool has no opinion set. */
export function baseAction(rules, toolKey) {
  const found = (rules ?? []).find((r) => r.tool === toolKey && r.pattern === ANY);
  return found?.action ?? null;
}

/** Everything narrower than the blanket rule, which is where the value is. */
export function patternRules(rules, toolKey) {
  return (rules ?? []).filter((r) => r.tool === toolKey && r.pattern !== ANY);
}

/** Replace the blanket rule. A null action removes it, leaving the tool unset. */
export function withBaseAction(rules, toolKey, action) {
  const rest = (rules ?? []).filter((r) => !(r.tool === toolKey && r.pattern === ANY));
  return action ? [...rest, { tool: toolKey, pattern: ANY, action }] : rest;
}

/** Replace every narrower rule for one tool. Order is not preserved because
 *  order does not decide anything: specificity does. */
export function withPatternRules(rules, toolKey, next) {
  const rest = (rules ?? []).filter((r) => !(r.tool === toolKey && r.pattern !== ANY));
  return [...rest, ...(next ?? [])];
}

/** Drop every rule a tool owns, so an agent falls back to what it inherits. */
export function withoutTool(rules, toolKey) {
  return (rules ?? []).filter((r) => r.tool !== toolKey);
}

/** True when this ruleset says anything at all about a tool. */
export function mentionsTool(rules, toolKey) {
  return (rules ?? []).some((r) => r.tool === toolKey);
}

/**
 * Every rule that matches a call, best first.
 *
 * `evaluate` returns only the winner, which is right for enforcement and wrong
 * for explaining: "this won because it beat two vaguer rules" is the sentence
 * that teaches someone how the ruleset behaves. Scored exactly as `evaluate`
 * scores, so the head of this list is always the rule that actually applies.
 */
export function rankedMatches(rules, tool, target = ANY) {
  return (rules ?? [])
    .filter((r) => matches(r.tool, tool) && matches(r.pattern, target))
    .map((r) => ({ rule: r, score: specificity(r.tool) * 100000 + specificity(r.pattern) }))
    .sort((a, b) => b.score - a.score);
}

/* -- the editor ---------------------------------------------------------- */

/**
 * The per-pattern rules for one tool.
 *
 * `rules` is only this tool's narrower rules; `onChange` hands back the whole
 * replacement list. Keeping the splice outside means the same editor drives the
 * workspace ruleset and an agent's override list without knowing which it is.
 */
export function RuleEditor({
  toolKey,
  label,
  rules = [],
  onChange,
  suggestions = [],
  placeholder = "Pattern, for example git push *",
  className,
}) {
  const patch = (index, next) =>
    onChange?.(rules.map((entry, i) => (i === index ? { ...entry, ...next } : entry)));

  const remove = (index) => onChange?.(rules.filter((_, i) => i !== index));

  const add = (entry) => onChange?.([...rules, entry]);

  // A suggestion the user already has is not a suggestion, it is a duplicate
  // waiting to confuse them about which one is in force.
  const unused = suggestions.filter((s) => !rules.some((r) => r.pattern === s.pattern));

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {rules.length ? (
        <div className="flex flex-col gap-1.5">
          {rules.map((entry, index) => (
            // Keyed by position, so only a row appended to the end is new
            // enough to slide in; editing or removing one leaves the rest put.
            <div key={index} className="flex animate-slide-up items-center gap-1.5">
              <Input
                size="xs"
                className="min-w-0 flex-1 font-mono"
                spellCheck={false}
                autoComplete="off"
                aria-label={`Pattern for ${label ?? toolKey}`}
                placeholder={placeholder}
                value={entry.pattern}
                onChange={(e) => patch(index, { pattern: e.target.value })}
              />
              <Segmented
                size="xs"
                label={`Action for ${entry.pattern || "this pattern"}`}
                value={entry.action}
                options={ACTION_OPTIONS}
                onChange={(action) => patch(index, { action })}
              />
              <IconButton
                size="sm"
                aria-label={`Remove the rule for ${entry.pattern || "this pattern"}`}
                onClick={() => remove(index)}
              >
                <X />
              </IconButton>
            </div>
          ))}
        </div>
      ) : null}

      <div className="flex flex-wrap items-center gap-1.5">
        <Button
          size="xs"
          variant="ghost"
          className="px-1.5 text-muted-foreground"
          onClick={() => add({ tool: toolKey, pattern: "", action: "ask" })}
        >
          <Plus />
          Add a rule
        </Button>
        {unused.map((suggestion) => (
          <Button
            key={suggestion.pattern}
            size="xs"
            variant="subtle"
            title={`${suggestion.pattern} is ${suggestion.action}`}
            onClick={() =>
              add({ tool: toolKey, pattern: suggestion.pattern, action: suggestion.action })
            }
          >
            <Plus />
            {suggestion.label}
          </Button>
        ))}
      </div>

      {rules.length > 1 ? (
        <p className="animate-fade-in text-[0.6875rem] leading-relaxed text-muted-foreground">
          Order does not matter. The rule with the most to say about a call is the one that decides
          it.
        </p>
      ) : null}
    </div>
  );
}
