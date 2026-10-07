import * as React from "react";
import { ANY, DEFAULT_ACTION, asRules, evaluate } from "@shared/permission";
import { defaultRules, groupedTools, keyForTool, toolByKey } from "@shared/tools";
import { useApp } from "@/lib/store";
import { tools as agentTools } from "@/lib/agent";
import { RotateCcw } from "@/components/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { Input } from "@/components/ui/input";
import { Select } from "@/components/ui/select";
import { useToast } from "@/components/ui/toast";
import { SettingsCard, SettingsField, SettingsSection } from "../SettingsRow";
import {
  ACTION_TONE,
  baseAction,
  patternRules,
  rankedMatches,
  withBaseAction,
  withPatternRules,
} from "./permissions/RuleEditor";
import { SessionGrants } from "./permissions/SessionGrants";
import { ToolRow } from "./permissions/ToolRow";

/**
 * The workspace ruleset: what every agent starts from.
 *
 * This screen edits the same array the backend enforces, which is the only
 * version of it worth having - a permission a person sets here and a permission
 * a tool checks at call time have to be literally the same record, or the screen
 * is a decoration that lies.
 *
 * It used to list the live tools a second time, in their own section, under
 * their own headings, with rows that either duplicated a control from above or
 * said "set under read" and offered nothing. Every live tool now sits inside the
 * row that governs it, so there is exactly one place to change any answer and
 * exactly one place to look for what an answer covers.
 *
 * The leftovers matter more than they look. A tool whose key is not in the
 * catalogue gets a row of its own at the bottom rather than being dropped: the
 * promise of this screen is that everything is controllable, and a tool nobody
 * wrote a heading for is precisely the one that would otherwise run unasked.
 */

function verdictWording(action) {
  if (action === "allow") return "runs without asking";
  if (action === "deny") return "is refused";
  return "stops and asks you first";
}

/** What a connected thing is, in the words a person would use for it. */
const SOURCE_LABELS = {
  builtin: "built in",
  mcp: "MCP",
  openapi: "API",
  composio: "Composio",
};

export function PermissionsPane() {
  const { permissions, setWorkspaceRules, threads } = useApp();
  const { toast } = useToast();

  // Anything that is not a list of rules came from an older build that stored a
  // map of invented capability ids. There is nothing in it worth migrating, so
  // it is treated as an empty ruleset rather than crashing the pane.
  const rules = React.useMemo(
    () => asRules(permissions?.workspace),
    [permissions]
  );
  const groups = React.useMemo(groupedTools, []);

  const [runtime, setRuntime] = React.useState([]);
  const [resetOpen, setResetOpen] = React.useState(false);

  // The live tool list only exists in the desktop app. A browser session gets
  // the catalogue and no coverage counts rather than an error.
  React.useEffect(() => {
    let alive = true;
    agentTools
      .list()
      .then((list) => alive && setRuntime(Array.isArray(list) ? list : []))
      .catch(() => alive && setRuntime([]));
    return () => {
      alive = false;
    };
  }, []);

  const setAction = (key, action) => setWorkspaceRules((prev) => withBaseAction(prev, key, action));
  const setPatterns = (key, next) => setWorkspaceRules((prev) => withPatternRules(prev, key, next));

  /* -- which live tools each key governs --------------------------------- */
  const byKey = React.useMemo(() => {
    const map = new Map();
    for (const tool of runtime) {
      const key = tool.key ?? keyForTool(tool.id);
      if (!map.has(key)) map.set(key, []);
      map.get(key).push({
        id: tool.id,
        description: tool.description || `A ${SOURCE_LABELS[tool.source] ?? tool.source} tool.`,
      });
    }
    for (const list of map.values()) list.sort((a, b) => a.id.localeCompare(b.id));
    return map;
  }, [runtime]);

  /**
   * Live tools whose key no permission in the catalogue owns.
   *
   * Normally empty. It is not decoration: a source that forgets to declare a
   * permission key lands here instead of running with no control at all, and
   * the row it gets is a real one.
   */
  const orphans = React.useMemo(
    () => [...byKey.keys()].filter((key) => !toolByKey(key)).sort(),
    [byKey]
  );

  /* -- try it ------------------------------------------------------------ */
  const subjects = React.useMemo(() => {
    const catalogue = groups.flatMap((group) =>
      group.tools.map((tool) => ({ value: tool.key, label: tool.label }))
    );
    const live = orphans.map((key) => ({ value: key, label: key }));
    const seen = new Set();
    return [...catalogue, ...live].filter((o) => !seen.has(o.value) && seen.add(o.value));
  }, [groups, orphans]);

  const [subject, setSubject] = React.useState("shell");
  const [target, setTarget] = React.useState("git push origin main");

  const trial = React.useMemo(() => {
    const value = target.trim() || ANY;
    return {
      verdict: evaluate(rules, subject, value),
      ranked: rankedMatches(rules, subject, value),
    };
  }, [rules, subject, target]);

  return (
    <div className="w-full">
      <SettingsSection flat
        title="What agents may do"
        description="The workspace ruleset. Every agent starts from this and can tighten or loosen any line of it on its own page. The most specific rule that matches a call is the one that decides it, so a narrow rule always beats a broad one no matter where it sits in the list."
      >
        {groups.map((group) => (
          <SettingsCard key={group.id} className="flex flex-col gap-0.5 p-1.5">
            <div className="px-1.5 pt-1 pb-1.5">
              <p className="text-[0.6875rem] font-semibold tracking-wide text-muted-foreground">
                {group.label}
              </p>
              <p className="mt-0.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
                {group.description}
              </p>
            </div>
            {group.tools.map((tool) => (
              <ToolRow
                key={tool.key}
                tool={tool}
                action={baseAction(rules, tool.key) ?? tool.fallback ?? DEFAULT_ACTION}
                onAction={(action) => setAction(tool.key, action)}
                patterns={patternRules(rules, tool.key)}
                onPatterns={(next) => setPatterns(tool.key, next)}
                suggestions={tool.suggestions ?? []}
                covers={byKey.get(tool.key) ?? []}
              />
            ))}
          </SettingsCard>
        ))}

        {orphans.length ? (
          <SettingsCard className="flex flex-col gap-0.5 p-1.5">
            <div className="px-1.5 pt-1 pb-1.5">
              <p className="text-[0.6875rem] font-semibold tracking-wide text-muted-foreground">
                Everything else
              </p>
              <p className="mt-0.5 text-[0.6875rem] leading-relaxed text-muted-foreground">
                Tools that ask under a name of their own rather than one of the headings above.
                Each is controlled here, by that name.
              </p>
            </div>
            {orphans.map((key) => (
              <ToolRow
                key={key}
                tool={{
                  key,
                  label: key,
                  description: byKey.get(key)?.[0]?.description ?? "No description.",
                  icon: "Wrench",
                  danger: "medium",
                }}
                action={baseAction(rules, key) ?? DEFAULT_ACTION}
                onAction={(action) => setAction(key, action)}
                patterns={patternRules(rules, key)}
                onPatterns={(next) => setPatterns(key, next)}
                covers={byKey.get(key) ?? []}
              />
            ))}
          </SettingsCard>
        ) : null}
      </SettingsSection>

      <SessionGrants
        threads={threads}
        onPromote={(grant) =>
          setWorkspaceRules((prev) => [
            ...prev.filter((r) => !(r.tool === grant.tool && r.pattern === grant.pattern)),
            { tool: grant.tool, pattern: grant.pattern, action: grant.action },
          ])
        }
      />

      <SettingsSection flat
        title="Try a call"
        description="Type what an agent might actually do and see which rule catches it, before it is a prompt in the middle of a turn."
      >
        <SettingsCard className="flex flex-col gap-2.5">
          <div className="flex flex-col gap-2 sm:flex-row sm:items-end">
            <SettingsField label="Permission" className="sm:w-52">
              <Select
                size="xs"
                ariaLabel="Permission to test"
                value={subject}
                onChange={setSubject}
                options={subjects}
              />
            </SettingsField>
            <SettingsField label="Command, path or argument" className="min-w-0 flex-1">
              <Input
                size="xs"
                className="font-mono"
                spellCheck={false}
                autoComplete="off"
                placeholder="git push origin main"
                value={target}
                onChange={(e) => setTarget(e.target.value)}
              />
            </SettingsField>
          </div>

          <div className="flex flex-col gap-1.5 rounded-lg card-surface-raised p-2.5">
            <p className="flex flex-wrap items-center gap-1.5 text-[13px] text-foreground">
              <Badge variant={ACTION_TONE[trial.verdict.action]} size="sm" dot>
                {trial.verdict.action}
              </Badge>
              <span>This {verdictWording(trial.verdict.action)}.</span>
            </p>
            {trial.verdict.rule ? (
              <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
                Decided by{" "}
                <span className="font-mono text-foreground">
                  {trial.verdict.rule.tool}
                  {trial.verdict.rule.pattern === ANY ? "" : ` ${trial.verdict.rule.pattern}`}
                </span>
                {trial.ranked.length > 1
                  ? `, which beat ${trial.ranked.length - 1} other matching ${
                      trial.ranked.length === 2 ? "rule" : "rules"
                    } because it is more specific.`
                  : ", the only rule that matches."}
              </p>
            ) : (
              <p className="text-[0.6875rem] leading-relaxed text-muted-foreground">
                Nothing in the ruleset matches, so the fallback applies and the agent asks first.
              </p>
            )}
            {trial.ranked.length > 1 ? (
              <ul className="flex animate-fade-in flex-col gap-0.5">
                {trial.ranked.slice(1, 4).map(({ rule }, i) => (
                  <li key={i} className="text-[0.6875rem] text-muted-foreground/80">
                    <span className="font-mono">
                      {rule.tool}
                      {rule.pattern === ANY ? "" : ` ${rule.pattern}`}
                    </span>{" "}
                    would have said {rule.action}
                  </li>
                ))}
              </ul>
            ) : null}
          </div>
        </SettingsCard>
      </SettingsSection>

      <SettingsSection flat title="Start over">
        <SettingsCard className="flex flex-wrap items-center justify-between gap-2">
          <p className="min-w-0 flex-1 text-[0.6875rem] leading-relaxed text-muted-foreground">
            Puts every permission back to the shipped answer: looking is allowed, changing the world
            asks. Exceptions you wrote are removed with it.
          </p>
          <Button size="xs" variant="danger" onClick={() => setResetOpen(true)}>
            <RotateCcw />
            Reset to defaults
          </Button>
        </SettingsCard>
      </SettingsSection>

      <ConfirmDialog
        open={resetOpen}
        onOpenChange={setResetOpen}
        destructive
        title="Reset the workspace ruleset?"
        description="Every rule on this screen is replaced by the shipped defaults. Rules an individual agent set for itself are not touched, and neither is anything remembered inside a conversation."
        confirmLabel="Reset"
        onConfirm={() => {
          setWorkspaceRules(defaultRules());
          setResetOpen(false);
          toast({ title: "Permissions reset", description: "Back to the shipped defaults." });
        }}
      />
    </div>
  );
}
