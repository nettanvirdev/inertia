import * as React from "react";
import { Check, Pencil, Pin, Trash2, Icon } from "@/components/icons";
import {
  LOW_CONFIDENCE_THRESHOLD,
  MEMORY_KIND_META,
  MEMORY_SOURCE_META,
  STALE_AFTER_DAYS,
  relativeTime,
} from "@/data";
import { Badge } from "@/components/ui/badge";
import { IconButton } from "@/components/ui/icon-button";
import { Tooltip } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/**
 * A memory is stale when it has not been recalled in STALE_AFTER_DAYS, counted
 * from the wall clock so the badge keeps meaning something as time passes.
 * `nowIso` is here for callers that need the answer pinned.
 */
export function isStale(memory, nowIso) {
  if (!memory?.lastUsedAt) return false;
  const cutoff =
    (nowIso == null ? Date.now() : new Date(nowIso).getTime()) - STALE_AFTER_DAYS * 86_400_000;
  return new Date(memory.lastUsedAt).getTime() < cutoff;
}

export function isLowConfidence(memory) {
  return (memory?.confidence ?? 1) < LOW_CONFIDENCE_THRESHOLD;
}

/**
 * Which project a memory belongs to, as a person would name it.
 *
 * The last segment of the path, because that is what somebody calls the project
 * and a full path in a chip would push everything else off the card. The whole
 * path is on the tooltip for anyone who has two folders with the same name.
 */
export function projectOf(memory) {
  if (memory?.scope !== "project" || !memory?.folder) return null;
  const parts = String(memory.folder)
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/);
  return parts[parts.length - 1] || null;
}

/** Three filled bars out of three - cheaper to read than a percentage. */
function Confidence({ value = 1 }) {
  const pct = Math.round(value * 100);
  const filled = value >= 0.85 ? 3 : value >= LOW_CONFIDENCE_THRESHOLD ? 2 : 1;
  return (
    <Tooltip content={`${pct}% confidence`}>
      <span className="flex items-center gap-1 text-[11px] text-muted-foreground">
        <span aria-hidden="true" className="flex items-end gap-px">
          {[0, 1, 2].map((i) => (
            <span
              key={i}
              className={cn(
                "w-1 rounded-full",
                i === 0 ? "h-1.5" : i === 1 ? "h-2" : "h-2.5",
                i < filled ? "bg-foreground/60" : "fill-track"
              )}
            />
          ))}
        </span>
        <span className="tabular-nums">{pct}%</span>
      </span>
    </Tooltip>
  );
}

export function MemoryCard({
  memory,
  compact = false,
  onPin,
  onDelete,
  onEdit,
  onOpen,
  onApprove,
  className,
  ...props
}) {
  if (!memory) return null;

  const kind = MEMORY_KIND_META[memory.kind] ?? { label: memory.kind, icon: "Circle" };
  const source = MEMORY_SOURCE_META[memory.source] ?? { label: memory.source, icon: "Circle" };
  const low = isLowConfidence(memory);
  const stale = isStale(memory);
  const project = projectOf(memory);

  const interactive = typeof onOpen === "function";

  return (
    <div
      role={interactive ? "button" : undefined}
      tabIndex={interactive ? 0 : undefined}
      onClick={interactive ? () => onOpen(memory) : undefined}
      onKeyDown={
        interactive
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onOpen(memory);
              }
            }
          : undefined
      }
      className={cn(
        // a surface step, never a border
        "group relative flex flex-col gap-2 rounded-2xl card-surface-subtle p-4 text-left",
        "transition-colors duration-150 ease-out",
        interactive &&
          "cursor-pointer outline-none hover:card-surface-raised focus-visible:fill-control-hover",
        className
      )}
      {...props}
    >
      {/* kind chip, then this memory's own state, then the hover actions */}
      <div className="flex items-start gap-1.5">
        <span className="inline-flex h-5 shrink-0 items-center gap-1 rounded-full fill-secondary px-2 text-[11px] font-medium text-muted-foreground">
          <Icon name={kind.icon} className="size-3" aria-hidden="true" />
          {kind.label}
        </span>

        {/* Approving is the one action never revealed on hover: a memory
            waiting for a decision is the reason the person came to this screen.
            It belongs beside the chip rather than above the right-hand edge,
            where the tick sat alone until hover brought the action row in under
            it and so read as a stray mark rather than a decision about this
            card.

            The pinned mark follows it because pinning follows saving - nothing
            can be pinned before it is kept - so the two read left to right as
            one story about where this memory stands. */}
        {memory.pending && onApprove ? (
          <Tooltip content="Let agents know this">
            <IconButton
              size="sm"
              label="Approve memory"
              onClick={(e) => {
                e.stopPropagation();
                onApprove(memory);
              }}
            >
              <Check className="size-3.5" />
            </IconButton>
          </Tooltip>
        ) : null}

        {memory.pinned ? (
          <Tooltip content="Pinned">
            <span className="grid size-5 shrink-0 place-items-center text-foreground/70">
              <Pin className="size-3.5 fill-current" aria-hidden="true" />
            </span>
          </Tooltip>
        ) : null}

        <span
          className={cn(
            "ml-auto flex shrink-0 items-center gap-0.5 opacity-0",
            "transition-opacity duration-150 ease-out",
            "group-hover:opacity-100 group-focus-within:opacity-100"
          )}
        >
          {onPin ? (
            <Tooltip content={memory.pinned ? "Unpin" : "Pin"}>
              <IconButton
                size="sm"
                label={memory.pinned ? "Unpin memory" : "Pin memory"}
                onClick={(e) => {
                  e.stopPropagation();
                  onPin(memory);
                }}
              >
                <Pin className={cn("size-3.5", memory.pinned && "fill-current")} />
              </IconButton>
            </Tooltip>
          ) : null}
          {onEdit ? (
            <Tooltip content="Edit">
              <IconButton
                size="sm"
                label="Edit memory"
                onClick={(e) => {
                  e.stopPropagation();
                  onEdit(memory);
                }}
              >
                <Pencil className="size-3.5" />
              </IconButton>
            </Tooltip>
          ) : null}
          {onDelete ? (
            <Tooltip content="Delete">
              <IconButton
                size="sm"
                label="Delete memory"
                className="hover:text-destructive-ink "
                onClick={(e) => {
                  e.stopPropagation();
                  onDelete(memory);
                }}
              >
                <Trash2 className="size-3.5" />
              </IconButton>
            </Tooltip>
          ) : null}
        </span>
      </div>

      <p className="text-[15px] leading-snug font-medium text-foreground">{memory.title}</p>

      <p
        className={cn(
          "text-[13px] leading-relaxed text-muted-foreground",
          compact && "line-clamp-3"
        )}
      >
        {memory.body}
      </p>

      {(low || stale || memory.pending) && (
        <div className="flex flex-wrap items-center gap-1.5">
          {/* Waiting for you to look at it. Written down, visible, and believed
              by nothing until it is approved - so it says so rather than
              sitting among the memories that are in force. */}
          {memory.pending ? (
            <Tooltip content="Written down but not in use yet. Approve it to let agents know it.">
              <span>
                <Badge variant="info" size="sm">
                  Waiting for you
                </Badge>
              </span>
            </Tooltip>
          ) : null}
          {low ? (
            <Tooltip
              content={`Confidence is ${Math.round(memory.confidence * 100)}% - verify this before relying on it.`}
            >
              <span>
                <Badge variant="warning" size="sm">
                  Low confidence
                </Badge>
              </span>
            </Tooltip>
          ) : null}
          {stale ? (
            <Tooltip
              content={`Not recalled since ${relativeTime(memory.lastUsedAt)} - over ${STALE_AFTER_DAYS} days ago.`}
            >
              <span>
                <Badge variant="warning" size="sm">
                  Stale
                </Badge>
              </span>
            </Tooltip>
          ) : null}
        </div>
      )}

      <div className="mt-auto flex flex-wrap items-center gap-x-3 gap-y-1 pt-1 text-[11px] text-muted-foreground">
        <span className="inline-flex items-center gap-1">
          <Icon name={source.icon} className="size-3" aria-hidden="true" />
          {source.label}
        </span>
        {/* Which project this belongs to, or nothing at all when it applies
            everywhere. Without it there is no way to tell from the screen why a
            memory is not being used in the folder you are looking at. */}
        {project ? (
          <Tooltip content={memory.folder}>
            <span className="inline-flex items-center gap-1">
              <Icon name="FolderOpen" className="size-3" aria-hidden="true" />
              {project}
            </span>
          </Tooltip>
        ) : (
          <span className="inline-flex items-center gap-1">
            <Icon name="Globe" className="size-3" aria-hidden="true" />
            Everywhere
          </span>
        )}
        <span className="tabular-nums">
          {memory.useCount} {memory.useCount === 1 ? "use" : "uses"}
        </span>
        <span>{relativeTime(memory.lastUsedAt)}</span>
        <span className="ml-auto">
          <Confidence value={memory.confidence} />
        </span>
      </div>
    </div>
  );
}
