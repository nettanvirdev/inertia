import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * A small hand-drawn icon set for the window chrome, ported from the Inertia
 * design system - 24x24 grid, 2px margin, stroke 1.5, round caps and joins,
 * so a titlebar glyph looks related to anything else drawn to the same DNA
 * later. `currentColor` throughout, so an icon is whatever ink its parent is.
 */

type Glyph = string[];

const GLYPHS = {
  Minus: ["M6.5 12h11"],
  Square: ["r 3.5 3.5 17 17 4.5"],
  X: ["M7.2 7.2 16.8 16.8", "M16.8 7.2 7.2 16.8"],
  Copy: [
    "r 8.5 8.5 12.5 12.5 3.5",
    "M15.5 5.5V5A2.5 2.5 0 0 0 13 2.5H5A2.5 2.5 0 0 0 2.5 5v8A2.5 2.5 0 0 0 5 15.5h.5",
  ],
} satisfies Record<string, Glyph>;

const NUM = /\s+/;

function renderElement(spec: string, i: number) {
  const filled = spec[0] === "!";
  const body = filled ? spec.slice(1) : spec;
  const paint = filled ? { fill: "currentColor", stroke: "none" } : undefined;
  const kind = body[0];

  if (kind === "c" && body[1] === " ") {
    const [cx, cy, r] = body.slice(2).trim().split(NUM);
    return <circle key={i} cx={cx} cy={cy} r={r} {...paint} />;
  }
  if (kind === "r" && body[1] === " ") {
    const [x, y, w, h, rx] = body.slice(2).trim().split(NUM);
    return <rect key={i} x={x} y={y} width={w} height={h} rx={rx} ry={rx} {...paint} />;
  }
  return <path key={i} d={body} {...paint} />;
}

function createIcon(name: string, glyph: Glyph) {
  const drawn = glyph.map(renderElement);

  const Component = React.forwardRef<SVGSVGElement, React.SVGProps<SVGSVGElement>>(
    function InertiaIcon({ className, ...props }, ref) {
      return (
        <svg
          ref={ref}
          viewBox="-1.2 -1.2 26.4 26.4"
          fill="none"
          stroke="currentColor"
          strokeLinecap="round"
          strokeLinejoin="round"
          className={cn("icon size-4 shrink-0", className)}
          {...props}
        >
          {drawn}
        </svg>
      );
    },
  );

  Component.displayName = name;
  return Component;
}

export const Minus = createIcon("Minus", GLYPHS.Minus);
export const Square = createIcon("Square", GLYPHS.Square);
export const X = createIcon("X", GLYPHS.X);
export const Copy = createIcon("Copy", GLYPHS.Copy);
