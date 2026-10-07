import * as React from "react";
import { cn } from "@/lib/utils";
import { Icon } from "@/components/icons";
import { useApp } from "@/lib/store";
import { ACTIVITY_CATEGORY_META, SEVERITY_META, relativeTime } from "@/data";
import { Badge } from "@/components/ui/badge";
import { Tooltip } from "@/components/ui/tooltip";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { absoluteTime } from "@/features/routines/run-status";
import { ROW } from "@/components/layout/View";

/** Severity → the badge variant it is allowed to spend. */
export const SEVERITY_BADGE = {
  info: "neutral",
  success: "success",
  warning: "warning",
  danger: "danger",
};

/**
 * One audit event. Renders as a button when the caller gives it an `onClick`
 * (the Activity feed opens a dialog), and as a plain row otherwise so it can be
 * embedded read-only inside an agent detail pane.
 */
export function ActivityRow({ event, showAgent = true, timeLabel, className, ...props }) {
  const { agents } = useApp();

  const severity = SEVERITY_META[event.severity] ?? SEVERITY_META.info;
  const category = ACTIVITY_CATEGORY_META[event.category] ?? {
    label: event.category,
    icon: "Circle",
  };
  const agent = showAgent && event.agentId ? agents.find((b) => b.id === event.agentId) : null;

  const interactive = typeof props.onClick === "function";
  const Root = interactive ? "button" : "div";

  return (
    <Root
      type={interactive ? "button" : undefined}
      className={cn(
        "group flex w-full items-start gap-3 text-left transition-colors",
        ROW,
        interactive && "outline-none hover:fill-nav focus-visible:fill-nav",
        className
      )}
      {...props}
    >
      <span
        aria-hidden="true"
        className="mt-0.5 grid size-8 shrink-0 place-items-center rounded-full"
        // The severity's own wash, which is a token. This was
        // `${severity.color}1a` - string-building an 8-digit hex - and it
        // worked only for as long as the colour was written as a hex.
        style={{ backgroundColor: severity.wash, color: severity.color }}
      >
        <Icon name={category.icon} className="size-3.5" />
      </span>

      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex min-w-0 items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-[13px] text-foreground">{event.title}</span>
          <Tooltip content={absoluteTime(event.at)}>
            <span className="shrink-0 text-[11px] tabular-nums text-muted-foreground">
              {timeLabel ?? relativeTime(event.at)}
            </span>
          </Tooltip>
        </span>

        <span className="truncate text-[11px] leading-relaxed text-muted-foreground">
          {event.detail}
        </span>

        <span className="mt-1 flex flex-wrap items-center gap-1.5">
          {agent ? (
            // pl matches py, so the round avatar sits concentric inside the round
            // pill; a symmetric px would leave a hollow crescent on the left,
            // because the 20px circle already fills the 24px pill vertically.
            <span className="inline-flex items-center gap-1.5 rounded-full fill-secondary py-0.5 pl-0.5 pr-2 text-[11px] text-muted-foreground">
              <AgentAvatar agent={agent} size="xs" showStatus={false} />
              {agent.name}
            </span>
          ) : null}
          <Badge variant={SEVERITY_BADGE[event.severity] ?? "neutral"} size="sm">
            {category.label}
          </Badge>
        </span>
      </span>
    </Root>
  );
}
