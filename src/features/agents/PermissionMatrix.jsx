import * as React from "react";
import { ANY, DEFAULT_ACTION, asRules, evaluate, merge } from "@shared/permission";
import { TOOLS, defaultRules, groupedTools } from "@shared/tools";
import { useApp } from "@/lib/store";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import {
  baseAction,
  mentionsTool,
  patternRules,
  withBaseAction,
  withPatternRules,
  withoutTool,
} from "@/features/settings/panes/permissions/RuleEditor";
import { ToolRow } from "@/features/settings/panes/permissions/ToolRow";

/**
 * One agent's answer to the workspace's question.
 *
 * The screen shows the EFFECTIVE ruleset - what this agent will actually do -
 * because that is the only number anyone is ever asking for. But an effective
 * value that hides where it came from is a trap: a person who tightens the
 * workspace and finds one agent unchanged needs to see that the agent said
 * something of its own, and needs one click to take it back. Hence the
 * override badge and Revert on every row that has one.
 *
 * Editing a row always writes into the agent's own list, never the workspace.
 * `merge` puts the agent last, so an override wins by construction.
 */
export function PermissionMatrix({ agentId, className }) {
  const { permissions, setAgentRules } = useApp();
  const groups = React.useMemo(groupedTools, []);

  // A workspace written by an older build stored a map of invented capability
  // ids here, not a ruleset. Nothing can be salvaged from it, and refusing to
  // render is worse than starting from the defaults, so anything that is not a
  // list of rules is ignored.
  const workspace = React.useMemo(
    () => merge(defaultRules(), asRules(permissions?.workspace)),
    [permissions]
  );
  const own = React.useMemo(() => asRules(permissions?.agents?.[agentId]), [permissions, agentId]);
  const effective = React.useMemo(() => merge(workspace, own), [workspace, own]);

  const setAction = (key, action) =>
    setAgentRules(agentId, (prev) => withBaseAction(prev, key, action));
  // Fork on write: the rows show the merged patterns, so touching one copies the
  // whole set onto the agent. That is the honest reading of "this agent has its
  // own answer now", and Revert undoes all of it in one click - the alternative,
  // an agent list that silently half-tracks the workspace, has no such undo.
  const setPatterns = (key, next) =>
    setAgentRules(agentId, (prev) => withPatternRules(prev, key, next));
  const revert = (key) => setAgentRules(agentId, (prev) => withoutTool(prev, key));

  /* -- the summary ------------------------------------------------------- */
  const summary = React.useMemo(() => {
    const counts = { allow: 0, ask: 0, deny: 0 };
    const destructive = [];
    for (const tool of TOOLS) {
      const action = evaluate(effective, tool.key, ANY).action;
      counts[action] = (counts[action] ?? 0) + 1;
      // What this agent can do without being stopped, among the tools where one
      // call is irreversible. This is the sentence someone is actually here for.
      if (tool.danger === "high" && action === "allow") destructive.push(tool.label);
    }
    return { counts, destructive };
  }, [effective]);

  return (
    <div className={cn("flex flex-col gap-5", className)}>
      <div className="flex flex-col gap-1.5">
        <div className="flex flex-wrap items-center gap-1.5">
          <Badge variant="success" size="sm">
            {summary.counts.allow} allowed
          </Badge>
          <Badge variant="warning" size="sm">
            {summary.counts.ask} ask
          </Badge>
          <Badge variant="danger" size="sm">
            {summary.counts.deny} denied
          </Badge>
          {own.length ? (
            <Badge variant="info" size="sm">
              {own.length} own {own.length === 1 ? "rule" : "rules"}
            </Badge>
          ) : null}
        </div>
        <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
          {summary.destructive.length
            ? `Nothing stops this agent from: ${summary.destructive.join(", ").toLowerCase()}. Narrow that with a rule rather than by denying the whole tool.`
            : "Everything irreversible stops and asks you first."}
        </p>
      </div>

      {groups.map((group) => (
        <section key={group.id} className="flex flex-col gap-1">
          <h3 className="px-1 text-[11px] font-semibold text-muted-foreground">{group.label}</h3>
          <div className="flex flex-col">
            {group.tools.map((tool) => {
              const overridden = mentionsTool(own, tool.key);
              const inherited = baseAction(workspace, tool.key) ?? tool.fallback ?? DEFAULT_ACTION;
              return (
                <ToolRow
                  key={tool.key}
                  tool={tool}
                  action={baseAction(effective, tool.key) ?? inherited}
                  onAction={(action) => setAction(tool.key, action)}
                  patterns={patternRules(effective, tool.key)}
                  onPatterns={(next) => setPatterns(tool.key, next)}
                  suggestions={tool.suggestions ?? []}
                  overridden={overridden}
                  onRevert={() => revert(tool.key)}
                  inheritedLabel={inherited}
                />
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
