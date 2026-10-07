import * as React from "react";
import { Avatar } from "@/components/ui/avatar";
import { Icon } from "@/components/icons";
import { useAvatarSrc } from "@/lib/avatar";
import { cn } from "@/lib/utils";

/**
 * The one place an agent record becomes an avatar. A picture wins over a glyph,
 * a glyph over initials; the fallback surface is tinted from `agent.avatarColor`
 * as a FLAT wash (never a gradient) so eight teammates stay tellable apart at
 * 20px.
 *
 * The picture is a file in the workspace - `agents/pictures/<id>.png` - read
 * back the same way the user's own photo is, so an agent given a picture by
 * another agent and one given a picture in the editor are the same thing.
 */

/** The glyph sits a step smaller than the ring, so the tint has room to read. */
const GLYPH = {
  xs: "size-3",
  sm: "size-3.5",
  md: "size-4",
  lg: "size-5",
  xl: "size-7",
};

/** `#6366f1` → `rgb(99 102 241 / a)`. Returns null for anything unparseable. */
function tint(hex, alpha) {
  if (typeof hex !== "string") return null;
  const raw = hex.replace("#", "");
  const full =
    raw.length === 3
      ? raw
          .split("")
          .map((c) => c + c)
          .join("")
      : raw;
  if (full.length !== 6 || /[^0-9a-f]/i.test(full)) return null;
  const r = parseInt(full.slice(0, 2), 16);
  const g = parseInt(full.slice(2, 4), 16);
  const b = parseInt(full.slice(4, 6), 16);
  return `rgb(${r} ${g} ${b} / ${alpha})`;
}

export function AgentAvatar({ agent, size = "md", showStatus = false, className, ...props }) {
  // Hooks first: an early return above one is a different number of hooks on
  // the render where an agent is missing, which React counts as an error.
  const picture = useAvatarSrc(agent);
  if (!agent) return null;

  const fill = tint(agent.avatarColor, 0.16);
  const ink = tint(agent.avatarColor, 1);

  return (
    <Avatar
      src={picture ?? undefined}
      name={agent.name}
      size={size}
      status={showStatus ? agent.status : undefined}
      icon={
        agent.icon ? (
          <Icon name={agent.icon} aria-hidden="true" className={GLYPH[size] ?? GLYPH.md} />
        ) : agent.initials ? (
          <span aria-hidden="true">{agent.initials}</span>
        ) : undefined
      }
      // the tint lands on the fallback surface only - an <img> covers it
      style={fill ? { "--agent-tint": fill, "--agent-ink": ink } : undefined}
      className={cn(
        fill &&
          "[&>span:first-child]:bg-[var(--agent-tint)] [&>span:first-child]:text-[var(--agent-ink)]",
        className
      )}
      {...props}
    />
  );
}
