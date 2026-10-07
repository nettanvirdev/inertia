import * as React from "react";
import { cn } from "@/lib/utils";

const SIZES = { sm: "size-3.5", md: "size-4", lg: "size-5" };

/** A 270° arc in currentColor - inherits the ink of whatever control it sits in. */
function Spinner({ size = "md", className, label = "Loading", ...props }) {
  return (
    <svg
      role="status"
      aria-label={label}
      viewBox="0 0 24 24"
      fill="none"
      className={cn("shrink-0 animate-spin-slow", SIZES[size] ?? SIZES.md, className)}
      {...props}
    >
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2.25" opacity="0.2" />
      <path
        d="M21 12a9 9 0 0 0-9-9"
        stroke="currentColor"
        strokeWidth="2.25"
        strokeLinecap="round"
      />
    </svg>
  );
}

export { Spinner };
