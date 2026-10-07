import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * The Inertia icon engine.
 *
 * Every glyph in the app is drawn here, from a compact geometry table, rather
 * than pulled from an icon library. Two reasons:
 *
 *   1. Weight. A named-import barrel of a third-party set cost ~600KB of JS.
 *      The whole hand-drawn set is a few kilobytes of path data.
 *   2. Voice. The design DNA is soft, rounded, no sharp corners - a stock set
 *      fights that on every screen. These are drawn to one grid and one
 *      stroke, so a 16px glyph beside a 16px glyph always looks related.
 *
 * Theme awareness is free: nothing here names a colour. Strokes and fills are
 * `currentColor`, so an icon is whatever ink its parent is - which is how
 * `text-muted-foreground` on a nav row dims the icon with the label, in both
 * palettes, with no per-theme icon variants to keep in sync.
 *
 * ── the grid ───────────────────────────────────────────────────────────────
 * 24×24 viewBox · 2px margin · 20×20 live area · stroke 1.5 · round caps and
 * joins · corner radii 3-5 · no corner tighter than 2.
 *
 * ── the shorthand ──────────────────────────────────────────────────────────
 * A glyph is an array of element strings. Prefix decides the primitive:
 *
 *   "M12 4v16"          path - anything starting with a command letter
 *   "c 12 12 5"         circle cx cy r
 *   "r 3 4 18 16 4"     rect x y w h rx
 *   "l 4 12 20 12"      line x1 y1 x2 y2
 *
 * A leading "!" fills the element with currentColor instead of stroking it -
 * for dots, pips and the solid core of a status glyph.
 */

const NUM = /\s+/;

function renderElement(spec, i) {
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
  if (kind === "l" && body[1] === " ") {
    const [x1, y1, x2, y2] = body.slice(2).trim().split(NUM);
    return <line key={i} x1={x1} y1={y1} x2={x2} y2={y2} {...paint} />;
  }
  return <path key={i} d={body} {...paint} />;
}

/**
 * Turn a geometry entry into a component with the shape every call site in the
 * app already expects: `className` for size and colour, everything else
 * forwarded. Sizing stays a class (`size-4`) so it keeps living in the
 * Tailwind scale rather than becoming a second, competing numeric prop.
 */
export function createIcon(name, glyph) {
  const drawn = glyph.map(renderElement);

  const Component = React.forwardRef(function InertiaIcon(
    { className, strokeWidth, ...props },
    ref
  ) {
    return (
      <svg
        ref={ref}
        // The glyphs are drawn on a 24 grid with a 2-unit margin, which fills
        // 83% of the box - noticeably fatter than a stock set, and it showed:
        // icons crowded their slots everywhere. Padding the viewBox instead of
        // rescaling 155 glyphs drops them to 76% and buys back the air. The
        // stroke weight in globals.css is scaled to match.
        viewBox="-1.2 -1.2 26.4 26.4"
        fill="none"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
        // `icon` carries the stroke weight from globals.css, so the whole set
        // can be re-weighted from the stylesheet without touching this file.
        className={cn("icon size-4 shrink-0", className)}
        {...props}
      >
        {drawn}
      </svg>
    );
  });

  Component.displayName = name;
  return Component;
}

/** Build a whole geometry table into components in one pass. */
export function createIcons(table) {
  const out = {};
  for (const key of Object.keys(table)) out[key] = createIcon(key, table[key]);
  return out;
}
