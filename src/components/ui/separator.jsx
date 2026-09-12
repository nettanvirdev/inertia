import { cn } from "@/lib/utils";

/**
 * A 1px line is the NARROW EXCEPTION, not the default. Separation in this system
 * comes from whitespace first, then from stepping the surface (`bg-card-lighter`
 * on the thing in front). Reach for this only where both genuinely fail - e.g.
 * the divider inside the composer toolbar.
 */
function Separator({ orientation = "horizontal", decorative = true, className, ...props }) {
  const vertical = orientation === "vertical";
  return (
    <div
      role={decorative ? "none" : "separator"}
      aria-orientation={decorative ? undefined : orientation}
      className={cn(
        "shrink-0 bg-border/60",
        vertical ? "h-full w-px" : "h-px w-full",
        className
      )}
      {...props}
    />
  );
}

export { Separator };
