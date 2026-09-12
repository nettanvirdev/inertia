import * as React from "react";
import { Play, Icon } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useApp } from "@/lib/store";
import { relativeTime } from "@/data";
import { Badge } from "@/components/ui/badge";
import { IconButton } from "@/components/ui/icon-button";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { Tooltip } from "@/components/ui/tooltip";
import { absoluteTime, runLabel, runTone } from "@/features/routines/run-status";
import { ROW } from "@/components/layout/View";

/**
 * One routine as a horizontal row. The left block is the navigation target; the
 * switch and the run action are siblings so a nested-button never happens.
 */
export function RoutineRow({ routine, compact = false, onOpen, className }) {
  const { toggleRoutine, runRoutine } = useApp();

  const last = routine.lastRun;
  const status = last?.status ?? null;
  const tone = runTone(status);
  const running = status === "running";
  const failing = status === "error";
  const next = routine.schedule?.nextRunAt ?? null;

  return (
    <div
      className={cn(
        "group flex animate-slide-up items-center gap-3 transition-colors hover:fill-nav",
        // Compact keeps its own tighter rhythm - it is the side panel's row,
        // where the pane is 20rem wide and the list is a summary rather than
        // the screen's subject.
        compact ? "-mx-2 rounded-xl px-2 py-1.5" : ROW,
        className
      )}
    >
      <button
        type="button"
        onClick={() => onOpen?.(routine.id)}
        className="flex min-w-0 flex-1 items-center gap-2.5 rounded-xl text-left outline-none focus-visible:fill-control-hover"
      >
        <span className="flex size-4 shrink-0 items-center justify-center" aria-hidden="true">
          {running ? (
            <Spinner size="sm" className="text-info-ink" label="Running" />
          ) : (
            <span className={cn("size-2 rounded-full", tone.dot)} />
          )}
        </span>

        <span
          className={cn(
            "grid size-7 shrink-0 place-items-center rounded-full fill-control transition-colors",
            routine.enabled ? "text-muted-foreground group-hover:text-foreground" : "text-muted-foreground/60"
          )}
          aria-hidden="true"
        >
          <Icon name={routine.icon} className="size-3.5" />
        </span>

        <span className="flex min-w-0 flex-1 flex-col">
          <span className="flex min-w-0 items-center gap-1.5">
            <span
              className={cn(
                "min-w-0 truncate text-[13.5px]",
                routine.enabled ? "font-medium text-foreground" : "text-muted-foreground",
                failing && "text-destructive-ink"
              )}
            >
              {routine.name}
            </span>
            {failing ? (
              <Badge variant="danger" size="sm">
                Failing
              </Badge>
            ) : null}
          </span>
          <span className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
            <span className="shrink-0">{routine.schedule?.humanLabel ?? "Unscheduled"}</span>
            {!compact && routine.description ? (
              <>
                <span aria-hidden="true" className="shrink-0 opacity-50">
                  ·
                </span>
                <span className="min-w-0 truncate">{routine.description}</span>
              </>
            ) : null}
          </span>
        </span>
      </button>

      <div className="flex shrink-0 items-center gap-2">
        {last ? (
          <Tooltip content={`${runLabel(status)} - ${absoluteTime(last.at)}`}>
            <span className={cn("hidden items-center gap-1.5 text-[11px] sm:inline-flex", tone.ink)}>
              <span className={cn("size-1.5 rounded-full", tone.dot)} aria-hidden="true" />
              {running ? "Running" : `${runLabel(status)} ${relativeTime(last.at)}`}
            </span>
          </Tooltip>
        ) : (
          <span className="hidden text-[11px] text-muted-foreground sm:inline">Never run</span>
        )}

        {!compact ? (
          <span className="hidden w-24 shrink-0 text-right text-[11px] text-muted-foreground lg:inline">
            {routine.enabled && next ? relativeTime(next) : "-"}
          </span>
        ) : null}

        <IconButton
          size="sm"
          label={`Run ${routine.name} now`}
          onClick={() => runRoutine(routine.id)}
          disabled={running}
          className="opacity-0 transition-opacity duration-150 group-hover:opacity-100 focus-visible:opacity-100"
        >
          <Play />
        </IconButton>

        <Switch
          size="sm"
          checked={Boolean(routine.enabled)}
          onCheckedChange={() => toggleRoutine(routine.id)}
          label={`${routine.enabled ? "Disable" : "Enable"} ${routine.name}`}
        />
      </div>
    </div>
  );
}
