/**
 * Core glyphs - the calibration batch.
 *
 * Everything else in the set is measured against these. Three decisions are
 * fixed here and inherited everywhere:
 *
 *   · A full-bleed round is r 8.5 on (12,12); a full-bleed box is rx 4.5.
 *   · Compact glyphs (check, plus, chevrons, arrows) live in a ~14px box so a
 *     Plus and a Globe read as the same optical size in the same 16px slot.
 *   · Every apex is blunted with a real arc, not left to the round join. A
 *     round join only softens a corner by half the stroke; an arrowhead or a
 *     checkmark elbow needs radius ~1.5 before it stops looking like a spike.
 *
 * The checkmark elbow and the arrowhead arc defined here are reused verbatim
 * inside every composite glyph in the other batches, so the tick in a shield is
 * the same tick as the tick in a menu row.
 */
export const GLYPHS_CORE = {
  // Shaft runs into the head's arc on purpose - the overlap keeps the join
  // solid at 12px, where a butt-to-butt meeting shows a notch.
  ArrowDown: ["M12 4.5v14.7", "M6.6 13.6 10.9 18.5a1.5 1.5 0 0 0 2.2 0l4.3-4.9"],
  ArrowUp: ["M12 19.5V4.8", "M6.6 10.4 10.9 5.5a1.5 1.5 0 0 1 2.2 0l4.3 4.9"],
  ArrowLeft: ["M19.5 12H4.8", "M10.4 6.6 5.5 10.9a1.5 1.5 0 0 0 0 2.2l4.9 4.3"],
  ArrowRight: ["M4.5 12h14.7", "M13.6 6.6 18.5 10.9a1.5 1.5 0 0 1 0 2.2l-4.9 4.3"],

  // THE checkmark. Reused, translated and scaled, by every composite tick.
  Check: ["M5.5 12.4 9 16a1.5 1.5 0 0 0 2.3-.1L18.5 7.6"],

  ChevronDown: ["M6.5 9.6 10.9 14.4a1.5 1.5 0 0 0 2.2 0l4.4-4.8"],
  ChevronUp: ["M6.5 14.4 10.9 9.6a1.5 1.5 0 0 1 2.2 0l4.4 4.8"],
  ChevronLeft: ["M14.4 6.5 9.6 10.9a1.5 1.5 0 0 0 0 2.2l4.8 4.4"],
  ChevronRight: ["M9.6 6.5 14.4 10.9a1.5 1.5 0 0 1 0 2.2l-4.8 4.4"],
  ChevronsLeft: [
    "M12.8 7.4 9 11a1.4 1.4 0 0 0 0 2l3.8 3.6",
    "M18.4 7.4 14.6 11a1.4 1.4 0 0 0 0 2l3.8 3.6",
  ],
  ChevronsRight: [
    "M11.2 7.4 15 11a1.4 1.4 0 0 1 0 2l-3.8 3.6",
    "M5.6 7.4 9.4 11a1.4 1.4 0 0 1 0 2l-3.8 3.6",
  ],

  Circle: ["c 12 12 8.5"],
  Square: ["r 3.5 3.5 17 17 4.5"],
  Minus: ["M6.5 12h11"],
  Plus: ["M12 6.5v11", "M6.5 12h11"],
  X: ["M7.2 7.2 16.8 16.8", "M16.8 7.2 7.2 16.8"],

  // The one glyph that is legitimately all pips.
  MoreHorizontal: ["!c 5.6 12 1.3", "!c 12 12 1.3", "!c 18.4 12 1.3"],

  CircleAlert: ["c 12 12 8.5", "M12 7.6v4.9", "!c 12 16 1.1"],
  CircleCheck: ["c 12 12 8.5", "M8.2 12.2 10.6 14.8a1.3 1.3 0 0 0 2-.1l3.6-4.6"],
  CircleX: ["c 12 12 8.5", "M9.3 9.3 14.7 14.7", "M14.7 9.3 9.3 14.7"],

  // Eight rounded shoulders rather than eight miters - an octagon is the one
  // shape in the set where hard corners would be most obvious.
  OctagonAlert: [
    "M9.1 2.9h5.8a1.5 1.5 0 0 1 1 .4l4.8 4.8a1.5 1.5 0 0 1 .4 1v5.8a1.5 1.5 0 0 1-.4 1l-4.8 4.8a1.5 1.5 0 0 1-1 .4H9.1a1.5 1.5 0 0 1-1-.4l-4.8-4.8a1.5 1.5 0 0 1-.4-1V9.1a1.5 1.5 0 0 1 .4-1l4.8-4.8a1.5 1.5 0 0 1 1-.4z",
    "M12 8v4.6",
    "!c 12 16 1.05",
  ],
  TriangleAlert: [
    "M10.3 3.9 2.7 17.1a2 2 0 0 0 1.7 3h15.2a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z",
    "M12 9.3v4",
    "!c 12 16.4 1.05",
  ],
  Info: ["c 12 12 8.5", "M12 11.3v4.9", "!c 12 8.2 1.1"],

  // Lens drawn as two mirrored curves so the corners of the eye are cusps of
  // real arcs; a single ellipse reads as a coin, not an eye.
  Eye: [
    "M21.6 12c-2.2 3.7-5.4 5.6-9.6 5.6S4.6 15.7 2.4 12c2.2-3.7 5.4-5.6 9.6-5.6s7.4 1.9 9.6 5.6z",
    "c 12 12 2.9",
  ],
  EyeOff: [
    "M6.4 8.6C4.6 9.6 3.3 10.7 2.4 12c2.2 3.7 5.4 5.6 9.6 5.6 1.6 0 3.1-.3 4.4-.8",
    "M9.9 6.6c.7-.1 1.4-.2 2.1-.2 4.2 0 7.4 1.9 9.6 5.6-.8 1.4-1.8 2.5-3 3.4",
    "M10 10.1a2.9 2.9 0 0 0 4 4.1",
    "M4.2 4.2 19.8 19.8",
  ],

  Search: ["c 10.8 10.8 7.3", "M16.2 16.2 20.8 20.8"],
  SearchX: ["c 10.8 10.8 7.3", "M16.2 16.2 20.8 20.8", "M8.7 8.7 12.9 12.9", "M12.9 8.7 8.7 12.9"],
  // A small square keystone with a three-quarter loop hung off each corner.
  // Each loop carries its own two outward strokes, so the square stays a clean
  // closed box and the loops can be tuned without redrawing it.
  Command: [
    "M9.25 9.25h5.5v5.5h-5.5z",
    "M9.25 9.25V6.5a2.75 2.75 0 1 0-2.75 2.75h2.75",
    "M14.75 9.25V6.5a2.75 2.75 0 1 1 2.75 2.75h-2.75",
    "M14.75 14.75v2.75a2.75 2.75 0 1 0 2.75-2.75h-2.75",
    "M9.25 14.75v2.75a2.75 2.75 0 1 1-2.75-2.75h2.75",
  ],

  Copy: [
    "r 8.5 8.5 12.5 12.5 3.5",
    "M15.5 5.5V5A2.5 2.5 0 0 0 13 2.5H5A2.5 2.5 0 0 0 2.5 5v8A2.5 2.5 0 0 0 5 15.5h.5",
  ],
  Download: [
    "M12 3.6v10.6",
    "M7.6 9.9 10.9 13.3a1.5 1.5 0 0 0 2.2 0l3.3-3.4",
    "M4 16.4V18a2.5 2.5 0 0 0 2.5 2.5h11A2.5 2.5 0 0 0 20 18v-1.6",
  ],
  ExternalLink: [
    "M10.5 4.2H6A2.5 2.5 0 0 0 3.5 6.7V18A2.5 2.5 0 0 0 6 20.5h11.3A2.5 2.5 0 0 0 19.8 18v-4.5",
    "M13.6 3.5h5.4a1.5 1.5 0 0 1 1.5 1.5v5.4",
    "M20 4 13.2 10.8",
  ],
  Maximize2: [
    "M14.5 3.7h4.3a1.5 1.5 0 0 1 1.5 1.5v4.3",
    "M9.5 20.3H5.2a1.5 1.5 0 0 1-1.5-1.5v-4.3",
    "M19.6 4.4 13.4 10.6",
    "M4.4 19.6 10.6 13.4",
  ],

  RotateCw: ["M20.5 12a8.5 8.5 0 1 1-2.6-6.1", "M20.8 3.6V7a1.4 1.4 0 0 1-1.4 1.4H16"],
  RotateCcw: ["M3.5 12a8.5 8.5 0 1 0 2.6-6.1", "M3.2 3.6V7a1.4 1.4 0 0 0 1.4 1.4H8"],

  // Every vertex of the triangle is an arc, so the play head reads as a guitar
  // pick rather than a shard.
  Play: [
    "M9.4 5.6a1.3 1.3 0 0 1 2-1.1l8.3 5.3a1.4 1.4 0 0 1 0 2.4l-8.3 5.3a1.3 1.3 0 0 1-2-1.1z",
  ],
  Pause: ["r 6.8 4.5 4 15 2", "r 13.2 4.5 4 15 2"],
  Loader: ["M12 3.5a8.5 8.5 0 1 0 8.5 8.5"],

  // The one brand mark in the set. Redrawn as an outline on our grid rather
  // than dropped in as a vendor silhouette, so it sits beside Globe and Cloud
  // in the integrations list without looking pasted in from elsewhere.
  Github: [
    "M15.2 20.8v-3a2.9 2.9 0 0 0-.8-2.2c2.7-.3 5.4-1.3 5.4-5.9a4.6 4.6 0 0 0-1.2-3.2 4.3 4.3 0 0 0-.1-3.2s-1-.3-3.3 1.2a11.3 11.3 0 0 0-6 0C6.9 3 5.9 3.3 5.9 3.3a4.3 4.3 0 0 0-.1 3.2 4.6 4.6 0 0 0-1.2 3.2c0 4.6 2.7 5.6 5.4 5.9a2.9 2.9 0 0 0-.8 2.2v3",
    "M9.2 18.9c-3.7 1.1-3.7-1.9-5.2-2.3",
  ],
};
