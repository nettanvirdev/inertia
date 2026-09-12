import * as React from "react";
import { Icon, Pencil, Trash2 } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { IconButton } from "@/components/ui/icon-button";

/**
 * One installed plugin, in the one shape all four tabs use.
 *
 * The whole row is a button so a click anywhere opens the editor; the switch
 * and the two trailing actions stop the event, because a user reaching for the
 * enable toggle has not asked for a dialog.
 */
export function PluginRow({ glyph, title, subtitle, badges = [], enabled, onToggle, onEdit, onRemove }) {
  const stop = (fn) => (e) => {
    e.stopPropagation();
    fn?.();
  };

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onEdit}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onEdit?.();
        }
      }}
      className={cn(
        "flex w-full animate-slide-up items-center gap-2.5 rounded-2xl fill-control px-3 py-2.5 text-left outline-none",
        "transition-colors duration-150 ease-out hover:fill-control-hover",
        "focus-visible:fill-control-hover",
        !enabled && "opacity-60"
      )}
    >
      <span className="grid size-7 shrink-0 place-items-center rounded-full fill-secondary text-muted-foreground">
        {typeof glyph === "string" ? (
          <Icon name={glyph} className="size-3.5" aria-hidden="true" />
        ) : (
          React.createElement(glyph, { className: "size-3.5", "aria-hidden": "true" })
        )}
      </span>

      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-[13px] text-foreground">{title}</span>
          {badges.map((badge) => (
            <Badge key={badge.label} size="sm" variant={badge.variant ?? "neutral"}>
              {badge.label}
            </Badge>
          ))}
        </div>
        {subtitle ? (
          <p className="truncate text-[11px] text-muted-foreground" title={subtitle}>
            {subtitle}
          </p>
        ) : null}
      </div>

      <span onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
        <Switch
          size="sm"
          checked={Boolean(enabled)}
          onCheckedChange={() => onToggle?.()}
          label={`${enabled ? "Disable" : "Enable"} ${title}`}
        />
      </span>

      <IconButton size="lg" label={`Edit ${title}`} onClick={stop(onEdit)}>
        <Pencil />
      </IconButton>
      <IconButton size="lg" label={`Remove ${title}`} onClick={stop(onRemove)}>
        <Trash2 />
      </IconButton>
    </div>
  );
}
