import * as React from "react";
import { MessageCircleQuestion, getIcon } from "@/components/icons";
import { cn } from "@/lib/utils";
import { ANY, asRules, evaluate, merge } from "@shared/permission";
import { toolByKey } from "@shared/tools";
import { useApp } from "@/lib/store";

/**
 * Opening moves for an empty thread, derived from what the agent can actually do
 * rather than a generic list - an agent with no browser should never be offered a
 * "look this up" prompt.
 */
const BY_TOOL = {
  grep: "Search the project for {topic} and tell me what you find.",
  glob: "Show me how this project is laid out.",
  read: "Walk me through the files you touched most recently.",
  edit: "Draft a change for {topic} and show me the diff first.",
  shell: "Show me what is running on this machine right now.",
  task: "Who would you hand this to, and what would you ask them?",
  skill: "Which of my skills apply to {topic}?",
};

/**
 * Openers built from the tools the agent may actually call.
 *
 * These used to come from an invented capability list, so an agent could be
 * offered "Take a look at the current screen" by a Desktop capability that no
 * tool in the app has ever provided. Now a prompt is only offered when the tool
 * behind it survives the agent's own permission rules, which is the same test
 * the tool registry applies when it builds the turn.
 */
function promptsFor(agent, rules) {
  if (!agent) return [];
  const topic = agent.tags?.[0] ?? "our roadmap";
  const list = [
    { id: "role", icon: "Sparkles", label: `What are you working on, ${agent.name}?` },
  ];
  for (const [key, template] of Object.entries(BY_TOOL)) {
    const verdict = evaluate(rules ?? [], key, ANY);
    if (verdict.action === "deny" && (verdict.rule?.pattern ?? ANY) === ANY) continue;
    list.push({
      id: key,
      icon: toolByKey(key)?.icon ?? "Circle",
      label: template.replace("{topic}", topic),
    });
    if (list.length >= 4) break;
  }
  return list;
}

export function StarterPrompts({ agent, onPick, className }) {
  const { permissions } = useApp();
  const rules = React.useMemo(
    () => merge(asRules(permissions?.workspace), asRules(permissions?.agents?.[agent?.id])),
    [permissions, agent?.id]
  );
  const prompts = React.useMemo(() => promptsFor(agent, rules), [agent, rules]);
  if (!prompts.length) return null;

  return (
    <div className={cn("flex flex-wrap gap-2", className)}>
      {prompts.map((prompt) => {
        const Glyph = prompt.icon ? getIcon(prompt.icon) : MessageCircleQuestion;
        return (
          <button
            key={prompt.id}
            type="button"
            onClick={() => onPick?.(prompt.label)}
            className={cn(
              // shrink-0 so a narrow column WRAPS the row instead of squeezing every
              // chip into a one-word-per-line tower; max-w-full keeps a chip that is
              // wider than the column inside it.
              "flex max-w-full shrink-0 items-center gap-2 rounded-xl fill-control px-3 py-2 text-left text-[13px] text-foreground",
              "outline-none transition-colors duration-150 ease-out",
              "hover:fill-control-hover focus-visible:fill-control-hover",
              "[&_svg]:size-3.5 [&_svg]:shrink-0 [&_svg]:text-muted-foreground hover:[&_svg]:text-foreground"
            )}
          >
            <Glyph aria-hidden="true" />
            <span className="min-w-0">{prompt.label}</span>
          </button>
        );
      })}
    </div>
  );
}
