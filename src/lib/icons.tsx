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
  Sun: [
    "c 12 12 4.2",
    "M12 2.6v2.4",
    "M12 19v2.4",
    "M4.3 4.3 6 6",
    "M18 18l1.7 1.7",
    "M2.6 12H5",
    "M19 12h2.4",
    "M4.3 19.7 6 18",
    "M18 6l1.7-1.7",
  ],
  Moon: ["M20.6 14.4A8.7 8.7 0 0 1 9.6 3.4 8.7 8.7 0 1 0 20.6 14.4Z"],
  Monitor: ["r 2.6 4 18.8 12.4 2.4", "M8.5 20.4h7", "M12 16.4v4"],
  ArrowRight: ["M4.5 12h14.2", "M13.2 6.2 19 12l-5.8 5.8"],
  ArrowLeft: ["M19.5 12H5.3", "M10.8 6.2 5 12l5.8 5.8"],
  Check: ["M5.2 12.6 9.7 17.2 18.8 7.4"],
  Sparkles: [
    "M10.4 3.4 12 7.9l4.5 1.6L12 11.1l-1.6 4.5-1.6-4.5L4.3 9.5 8.8 7.9Z",
    "M18 14.2l.9 2.4 2.4.9-2.4.9-.9 2.4-.9-2.4-2.4-.9 2.4-.9Z",
  ],
  Folder: ["M2.8 7A2.2 2.2 0 0 1 5 4.8h3.6l2.2 2.6H19A2.2 2.2 0 0 1 21.2 9.6v7.6A2.2 2.2 0 0 1 19 19.4H5A2.2 2.2 0 0 1 2.8 17.2Z"],
  Download: ["M12 3.6v11.2", "M7.6 10.6 12 15l4.4-4.4", "M4.4 18.6h15.2"],
  Loader: ["M12 3.4a8.6 8.6 0 1 0 8.6 8.6"],
  Alert: ["M12 4.2 21 19.6H3Z", "M12 10v4", "!c 12 17 0.9"],
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
export const Sun = createIcon("Sun", GLYPHS.Sun);
export const Moon = createIcon("Moon", GLYPHS.Moon);
export const Monitor = createIcon("Monitor", GLYPHS.Monitor);
export const ArrowRight = createIcon("ArrowRight", GLYPHS.ArrowRight);
export const ArrowLeft = createIcon("ArrowLeft", GLYPHS.ArrowLeft);
export const Check = createIcon("Check", GLYPHS.Check);
export const Sparkles = createIcon("Sparkles", GLYPHS.Sparkles);
export const Folder = createIcon("Folder", GLYPHS.Folder);
export const Download = createIcon("Download", GLYPHS.Download);
export const Loader = createIcon("Loader", GLYPHS.Loader);
export const Alert = createIcon("Alert", GLYPHS.Alert);
