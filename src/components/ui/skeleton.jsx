import * as React from "react";
import { cn } from "@/lib/utils";

/** Skeletons mirror layout, not shapes - size them to the real content's box. */
function Skeleton({ className, ...props }) {
  return (
    <div
      aria-hidden="true"
      className={cn("animate-soft-pulse rounded-md bg-muted", className)}
      {...props}
    />
  );
}

function SkeletonRow({ lines = 2, className, ...props }) {
  // last line runs short so a block of rows never reads as a grey slab
  const widths = ["w-3/4", "w-full", "w-5/6", "w-2/3"];
  return (
    <div
      role="status"
      aria-busy="true"
      aria-label="Loading"
      className={cn("flex w-full flex-col gap-2", className)}
      {...props}
    >
      {Array.from({ length: lines }).map((_, i) => (
        <Skeleton
          key={i}
          className={cn("h-3.5", i === lines - 1 ? "w-1/2" : widths[i % widths.length])}
        />
      ))}
    </div>
  );
}

export { Skeleton, SkeletonRow };
