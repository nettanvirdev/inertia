import * as React from "react";
import { Eye, EyeOff, Icon, Pencil, Trash2 } from "@/components/icons";
import { cn } from "@/lib/utils";
import { useDateFormat } from "@/lib/datetime";
import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/ui/copy-button";
import { IconButton } from "@/components/ui/icon-button";
import { Tooltip } from "@/components/ui/tooltip";

/**
 * One stored key, as a row.
 *
 * There is exactly one of these because there was very nearly two. The Composio
 * key arrived as a card of its own - a paste field, a Save button, a paragraph
 * under it - which said the same things a row says and said them four times
 * taller. A key is a key: its name, whether it is set, enough of it to
 * recognise, and the three things you do to one. The card only ever earned its
 * height by holding a Test button, and a Test button fits on a row.
 *
 * The actions are props rather than assumptions, because a key the app needs by
 * name cannot be renamed (the backend looks it up by that name) and should
 * not be deletable by accident, while a key you added yourself is yours to
 * remove. Passing no `onRemove` is how a row says so.
 */
export function SecretRow({
  /** The stored record, or null for a known key that has not been set yet. */
  entry,
  /** The key's name. Taken from `entry` when there is one. */
  name,
  /** What this key is for, when the row wants to say more than the label does. */
  purpose,
  icon = "KeyRound",
  revealed = false,
  value,
  onToggle,
  onCopy,
  onEdit,
  onRemove,
  /** Anything that belongs before the reveal - a Test button, usually. */
  actions,
  /** A line under the name, replacing the label. A test result, usually. */
  note,
}) {
  // The date the user chose to read dates in, rather than whatever the browser
  // guessed - see lib/datetime.
  const { formatDate } = useDateFormat();
  const keyName = entry?.name ?? name;

  const sub =
    note ??
    [purpose || entry?.label, entry?.updatedAt ? `Updated ${formatDate(entry.updatedAt)}` : null]
      .filter(Boolean)
      .join(" · ");

  return (
    <div className="flex animate-slide-up items-center gap-2 rounded-lg px-2 py-1.5 transition-colors duration-150 ease-out hover:fill-control-hover">
      <Icon
        name={icon}
        className="size-3.5 shrink-0 text-muted-foreground"
        aria-hidden="true"
      />

      <div className="min-w-0 flex-1">
        <p className="truncate font-mono text-xs text-foreground">{keyName}</p>
        <p className="truncate text-[0.6875rem] leading-tight text-muted-foreground">
          {sub || "No description"}
        </p>
      </div>

      {entry ? (
        // Keyed on the reveal so the value fades in over the hint rather
        // than the characters changing under the eye.
        <p
          key={revealed ? "value" : "hint"}
          className={cn(
            "hidden max-w-[16rem] shrink-0 animate-fade-in truncate font-mono text-[0.6875rem] sm:block",
            revealed ? "text-foreground" : "text-muted-foreground"
          )}
        >
          {revealed ? (value ?? "") : entry.hint}
        </p>
      ) : (
        <Badge size="sm">Not set</Badge>
      )}

      {actions}

      {entry ? (
        <>
          <Tooltip content={revealed ? "Hide value" : "Reveal value"}>
            <IconButton
              size="sm"
              label={revealed ? `Hide ${keyName}` : `Reveal ${keyName}`}
              onClick={onToggle}
            >
              {revealed ? <EyeOff /> : <Eye />}
            </IconButton>
          </Tooltip>
          <CopyButton label={`Copy ${keyName}`} onCopy={onCopy} />
        </>
      ) : null}

      {onEdit ? (
        <Tooltip content={entry ? "Replace" : "Add the key"}>
          <IconButton size="sm" label={`Edit ${keyName}`} onClick={onEdit}>
            <Pencil />
          </IconButton>
        </Tooltip>
      ) : null}

      {onRemove ? (
        <Tooltip content="Remove">
          <IconButton size="sm" label={`Remove ${keyName}`} onClick={onRemove}>
            <Trash2 />
          </IconButton>
        </Tooltip>
      ) : null}
    </div>
  );
}
