import * as React from "react";
import { cva } from "class-variance-authority";
import { cn } from "@/lib/utils";

// Never a solid saturated slab - the accent at 10% fill with the accent as ink.
const badgeVariants = cva(
  "inline-flex shrink-0 items-center justify-center gap-1 rounded-full leading-none whitespace-nowrap",
  {
    variants: {
      variant: {
        neutral: "fill-secondary text-muted-foreground",
        success: "bg-success-wash text-success-ink",
        warning: "bg-warning-wash text-warning-ink",
        danger: "bg-destructive-wash text-destructive-ink",
        info: "bg-info-wash text-info-ink",
      },
      size: {
        sm: "h-4 px-1.5 text-[10px] font-semibold",
        md: "h-5 px-2 text-[11px] font-medium",
      },
    },
    defaultVariants: { variant: "neutral", size: "md" },
  }
);

const DOT = {
  neutral: "bg-muted-foreground",
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-destructive",
  info: "bg-info",
};

function Badge({ variant = "neutral", size = "md", dot = false, className, children, ...props }) {
  return (
    <span className={cn(badgeVariants({ variant, size }), className)} {...props}>
      {dot ? (
        <span
          aria-hidden="true"
          className={cn("size-1.5 shrink-0 rounded-full", DOT[variant] ?? DOT.neutral)}
        />
      ) : null}
      {children}
    </span>
  );
}

export { Badge, badgeVariants };
