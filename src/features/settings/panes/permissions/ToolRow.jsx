import * as React from "react";
import { ChevronRight, Icon, RotateCcw, TriangleAlert } from "@/components/icons";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapse } from "@/components/ui/collapse";
import { Segmented } from "@/components/ui/segmented";
import { cn } from "@/lib/utils";
import { ACTION_OPTIONS, RuleEditor } from "./RuleEditor";

/**
 * One permission, and everything a person can say about it.
 *
 * The three-way control is the blunt answer. The exceptions under it are the
 * real one, and the tools it governs are the proof that it reaches something.
 * Both used to be spread across other sections of the screen: exceptions sat
 * permanently open under all thirteen rows at once, and the tools were listed a
 * second time further down under headings of their own. That is the same
 * information twice, and thirteen open editors is not a screen anybody reads.
 *
 * So both fold into the row that owns them, closed by default, with a line of
 * summary that makes opening them a choice rather than a search. A collapsed
 * row still says how many exceptions it has and what they do, because the
 * common question is "did I write something here", not "what exactly".
 *
 * The row knows nothing about where its answer is stored. The workspace pane
 * hands it the workspace ruleset, an agent's page hands it the effective one
 * plus a way to revert, and neither has to grow a second copy of this markup.
 */

/** A closed row still has to say what is inside it. */
function summarise(patterns) {
  return patterns
    .slice(0, 3)
    .map((entry) => `${entry.pattern || "(empty)"} ${entry.action}`)
    .join(" · ");
}

function Disclosure({ open, onToggle, children, label }) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={open}
      aria-label={label}
      className={cn(
        "flex items-center gap-1 rounded-lg px-1.5 py-0.5 outline-none",
        "text-[0.6875rem] text-muted-foreground",
        "transition-colors duration-150 ease-out hover:fill-control-hover hover:text-foreground",
        "focus-visible:fill-control-hover"
      )}
    >
      <ChevronRight
        className={cn(
          "size-3 shrink-0 transition-transform duration-[var(--motion-fast)] ease-[var(--ease-out)]",
          open && "rotate-90"
        )}
        aria-hidden="true"
      />
      {children}
    </button>
  );
}

export function ToolRow({
  tool,
  action,
  onAction,
  patterns = [],
  onPatterns,
  suggestions = [],
  /** The live tools that ask under this key. Proof the setting reaches something. */
  covers = [],
  overridden = false,
  onRevert,
  inheritedLabel,
  className,
}) {
  const denied = action === "deny";
  const high = tool.danger === "high";

  const [showRules, setShowRules] = React.useState(false);
  const [showCovers, setShowCovers] = React.useState(false);

  return (
    <div
      className={cn(
        "group flex flex-col gap-1.5 rounded-xl px-2.5 py-2",
        "transition-colors duration-150 ease-out hover:fill-nav",
        className
      )}
    >
      <div className="flex items-start gap-3">
        <span
          aria-hidden="true"
          className={cn(
            "mt-0.5 flex shrink-0 items-center text-muted-foreground",
            "transition-colors duration-150 ease-out group-hover:text-foreground",
            denied && "opacity-50"
          )}
        >
          <Icon name={tool.icon ?? "Wrench"} className="size-4" />
        </span>

        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-1.5">
            <span className={cn("text-[13px] text-foreground", denied && "text-muted-foreground")}>
              {tool.label}
            </span>
            {/* Loud only for the tools where one call is irreversible. A badge
                on everything is a badge on nothing. */}
            {high ? (
              <Badge variant="danger" size="sm" className="gap-1">
                <TriangleAlert className="size-2.5" />
                Destructive
              </Badge>
            ) : null}
            {overridden ? (
              <Badge variant="info" size="sm">
                Overridden
              </Badge>
            ) : null}
          </p>
          <p
            className={cn(
              "mt-0.5 text-[0.6875rem] leading-relaxed text-muted-foreground",
              denied && "opacity-70"
            )}
          >
            {tool.description}
          </p>
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          {overridden && onRevert ? (
            <Button
              size="xs"
              variant="ghost"
              className="px-1.5 text-muted-foreground"
              title={
                inheritedLabel
                  ? `Go back to the workspace setting, ${inheritedLabel}`
                  : "Go back to the workspace setting"
              }
              onClick={onRevert}
            >
              <RotateCcw />
              Revert
            </Button>
          ) : null}
          <Segmented
            size="xs"
            label={tool.label}
            value={action ?? "ask"}
            options={ACTION_OPTIONS}
            onChange={onAction}
          />
        </div>
      </div>

      {/* the two disclosures, on one line, aligned under the description */}
      {onPatterns || covers.length ? (
        <div className="flex flex-wrap items-center gap-1 pl-7">
          {onPatterns ? (
            <Disclosure
              open={showRules}
              onToggle={() => setShowRules((v) => !v)}
              label={`${showRules ? "Hide" : "Show"} exceptions for ${tool.label}`}
            >
              {patterns.length
                ? `${patterns.length} ${patterns.length === 1 ? "exception" : "exceptions"}`
                : "Add an exception"}
            </Disclosure>
          ) : null}
          {covers.length ? (
            <Disclosure
              open={showCovers}
              onToggle={() => setShowCovers((v) => !v)}
              label={`${showCovers ? "Hide" : "Show"} the tools governed by ${tool.label}`}
            >
              {covers.length === 1 ? "1 tool" : `${covers.length} tools`}
            </Disclosure>
          ) : null}
          {patterns.length && !showRules ? (
            <span className="min-w-0 truncate pl-1 font-mono text-[0.6875rem] text-muted-foreground/80">
              {summarise(patterns)}
            </span>
          ) : null}
        </div>
      ) : null}

      {onPatterns ? (
        <Collapse open={showRules}>
          <RuleEditor
            className="pl-7"
            toolKey={tool.key}
            label={tool.label}
            rules={patterns}
            onChange={onPatterns}
            suggestions={suggestions}
          />
        </Collapse>
      ) : null}

      {covers.length ? (
        <Collapse open={showCovers}>
          <ul className="flex flex-col gap-0.5 pl-7">
            {covers.map((entry) => (
              <li key={entry.id} className="flex items-baseline gap-2">
                <span className="shrink-0 font-mono text-[0.6875rem] text-foreground/90">
                  {entry.id}
                </span>
                <span className="min-w-0 flex-1 truncate text-[0.6875rem] text-muted-foreground">
                  {entry.description}
                </span>
              </li>
            ))}
          </ul>
        </Collapse>
      ) : null}
    </div>
  );
}
