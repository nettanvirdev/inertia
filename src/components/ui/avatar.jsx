import * as React from "react";
import { cn } from "@/lib/utils";

const SIZES = {
  xs: { box: "size-5", text: "text-[9px]", icon: "[&_svg]:size-3", dot: "size-1.5" },
  sm: { box: "size-6", text: "text-[10px]", icon: "[&_svg]:size-3.5", dot: "size-2" },
  md: { box: "size-8", text: "text-xs", icon: "[&_svg]:size-4", dot: "size-2.5" },
  lg: { box: "size-10", text: "text-sm", icon: "[&_svg]:size-5", dot: "size-2.5" },
  xl: { box: "size-14", text: "text-lg", icon: "[&_svg]:size-6", dot: "size-3" },
};

const STATUS = {
  online: "bg-success",
  busy: "bg-warning",
  idle: "bg-muted-foreground",
  offline: "bg-muted",
};

function initials(name) {
  if (!name) return "";
  const words = String(name).trim().split(/\s+/).filter(Boolean);
  if (!words.length) return "";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[words.length - 1][0]).toUpperCase();
}

function Avatar({ src, name, size = "md", status, icon, className, ...props }) {
  const s = SIZES[size] ?? SIZES.md;
  const [broken, setBroken] = React.useState(false);
  React.useEffect(() => setBroken(false), [src]);

  return (
    <span className={cn("relative inline-flex shrink-0", s.box, className)} {...props}>
      <span
        className={cn(
          "flex size-full select-none items-center justify-center overflow-hidden rounded-full",
          "bg-muted font-medium text-muted-foreground",
          s.text,
          s.icon
        )}
      >
        {src && !broken ? (
          <img
            src={src}
            alt={name || ""}
            onError={() => setBroken(true)}
            className="size-full object-cover"
          />
        ) : icon ? (
          icon
        ) : (
          <span aria-hidden={!name}>{initials(name)}</span>
        )}
      </span>
      {status ? (
        <span
          role="img"
          aria-label={status}
          // ring-background is the one legal ring: it punches the dot out of the
          // avatar edge so two adjacent circles never merge.
          className={cn(
            "absolute right-0 bottom-0 rounded-full ring-2 ring-background",
            s.dot,
            STATUS[status] ?? STATUS.offline
          )}
        />
      ) : null}
    </span>
  );
}

function AvatarGroup({ children, max = 4, size = "md", className, ...props }) {
  const items = React.Children.toArray(children).filter(Boolean);
  const shown = max != null ? items.slice(0, max) : items;
  const overflow = items.length - shown.length;
  const s = SIZES[size] ?? SIZES.md;

  return (
    <div className={cn("flex items-center -space-x-1.5", className)} {...props}>
      {shown.map((child, i) => (
        // inline-flex, not the default inline: an inline box is sized from the
        // line box, so the ring drew a squat rounded rectangle around the
        // avatar instead of tracing its circle.
        <span key={i} className="inline-flex rounded-full ring-2 ring-background">
          {child}
        </span>
      ))}
      {overflow > 0 ? (
        <span
          className={cn(
            "flex select-none items-center justify-center rounded-full",
            "bg-muted font-medium text-muted-foreground ring-2 ring-background",
            s.box,
            s.text
          )}
        >
          +{overflow}
        </span>
      ) : null}
    </div>
  );
}

export { Avatar, AvatarGroup };
