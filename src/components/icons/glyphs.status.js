/**
 * Status glyphs - time, theme, safety and craft.
 *
 * Four motifs are fixed here and reused inside this batch:
 *
 *   · The theme family (Sun / SunMoon / Sunrise / Moon) shares one disc radius
 *     (4 on 12,12), one ray run (r 6.6 → r 8.7, eight of them) and one crescent
 *     profile - the Moon's crescent scaled by 4/8.5 becomes SunMoon's, so the
 *     two read as the same moon at two sizes when they sit side by side in the
 *     theme switcher.
 *   · One shield silhouette, blunted at the apex and at the bottom tip, carries
 *     both ShieldCheck and ShieldAlert. The tick is core `CircleCheck`'s elbow
 *     scaled 0.85; the bang is core `CircleAlert`'s bar-plus-pip scaled 0.85.
 *   · One clock - a ring with an L-hand whose elbow is a real arc - is shared
 *     by Timer (ring on 12,13.5) and CalendarClock (same hand, scaled 0.59).
 *   · Settings is a 6-tooth gear whose tips AND roots are arcs on the r 8.5 /
 *     r 6.2 circles, so nothing on it is a miter. Settings2 (three tracks) and
 *     SlidersHorizontal (two tracks) stay deliberately far apart in density.
 */
export const GLYPHS_STATUS = {
  // ── being told something ────────────────────────────────────────────────
  // The bell is one continuous shoulder rather than a dome on a skirt: the
  // body rises from the rim at 4.4,16.2 on a single arc to the crown at 12,3.6
  // and back down, so there is no corner anywhere on it. The rim is a straight
  // run because a bell that curved at the mouth would read as a balloon, and
  // the clapper is a 2.2-wide cradle hung below it on the same radius the
  // shoulder ends at.
  Bell: [
    "M5.4 16.4a1 1 0 0 1-0.8-1.6c0.8-1.1 1.4-2 1.4-4.4a6 6 0 1 1 12 0c0 2.4 0.6 3.3 1.4 4.4a1 1 0 0 1-0.8 1.6Z",
    "M10.1 19.4a2.2 2.2 0 0 0 3.8 0",
  ],
  // The same bell with the shoulder cut where the slash crosses it, so the two
  // read as one glyph switched off rather than as a bell with a line on top.
  BellOff: [
    "M17.6 14.2c0.3 0.9 0.7 1.5 1 2a1 1 0 0 1-0.8 1.6H7.2",
    "M7.2 7.4A6 6 0 0 1 18 10.4c0 0.7 0 1.3 0.1 1.8",
    "M6 10.4c0 2.4-0.6 3.3-1.4 4.4a1 1 0 0 0 0.8 1.6",
    "M10.1 19.4a2.2 2.2 0 0 0 3.8 0",
    "M3.4 3.4 20.6 20.6",
  ],

  // ── time ────────────────────────────────────────────────────────────────
  // Calendar body is left open at the bottom-right so the clock ring sits in
  // the gap rather than crossing the frame.
  CalendarClock: [
    "M18.6 11.8V6.8A2.6 2.6 0 0 0 16 4.2H5.2A2.6 2.6 0 0 0 2.6 6.8v7.2A2.6 2.6 0 0 0 5.2 16.6h5.6",
    "M2.6 8.8h16",
    "M7.4 2.6v3.2 M14 2.6v3.2",
    "c 17.9 17.9 3.9",
    "M17.9 15.8v1.4a0.7 0.7 0 0 0 0.7 0.7h0.9",
  ],
  Timer: ["M9.4 3.6h5.2", "c 12 13.5 7", "M12 9.9v2.5a1.1 1.1 0 0 0 1.1 1.1h1.6"],

  // ── theme family ────────────────────────────────────────────────────────
  // Eight rays as one path: disc r 4, rays r 6.6 → r 8.7. Every other glyph in
  // the family is derived from these two numbers.
  Sun: [
    "c 12 12 4",
    "M12 3.3v2.1 M12 18.6v2.1 M3.3 12h2.1 M18.6 12h2.1 M5.9 5.9 7.3 7.3 M16.7 16.7 18.1 18.1 M18.1 5.9 16.7 7.3 M5.9 18.1 7.3 16.7",
  ],
  // Outer arc on r 8.5, cut by an r 6.5 arc centred up-right - crescent is
  // ~3.8 thick at its fattest.
  Moon: ["M11.4 3.5A8.5 8.5 0 1 0 20.5 12.6 6.5 6.5 0 0 1 11.4 3.5z"],
  // The Moon crescent scaled 4/8.5 and dropped into the Sun's disc slot, with
  // the Sun's rays untouched.
  SunMoon: [
    "M11.7 8A4 4 0 1 0 16 12.3 3.1 3.1 0 0 1 11.7 8z",
    "M12 3.3v2.1 M12 18.6v2.1 M3.3 12h2.1 M18.6 12h2.1 M5.9 5.9 7.3 7.3 M16.7 16.7 18.1 18.1 M18.1 5.9 16.7 7.3 M5.9 18.1 7.3 16.7",
  ],
  // Same disc and same ray run, recentred on (12,16) and cut at the horizon;
  // only the five rays above the horizon survive.
  Sunrise: [
    "M8 16a4 4 0 0 1 8 0",
    "M3.3 16h2.1 M18.6 16h2.1 M12 7.3v2.1 M5.9 9.9 7.3 11.3 M18.1 9.9 16.7 11.3",
    "M4.2 19.6h15.6",
  ],

  // ── safety ──────────────────────────────────────────────────────────────
  // THE shield. Apex and bottom tip are both real arcs, not joins.
  ShieldCheck: [
    "M11.1 2.8a2.2 2.2 0 0 1 1.8 0l6.5 2.7a1.9 1.9 0 0 1 1.2 1.8v4.3c0 4.4-2.9 7.6-7.5 9.6a2.4 2.4 0 0 1-2.2 0c-4.6-2-7.5-5.2-7.5-9.6V7.3a1.9 1.9 0 0 1 1.2-1.8z",
    "M8.6 11.6 10.6 13.8a1.1 1.1 0 0 0 1.7-0.1l3.1-3.9",
  ],
  ShieldAlert: [
    "M11.1 2.8a2.2 2.2 0 0 1 1.8 0l6.5 2.7a1.9 1.9 0 0 1 1.2 1.8v4.3c0 4.4-2.9 7.6-7.5 9.6a2.4 2.4 0 0 1-2.2 0c-4.6-2-7.5-5.2-7.5-9.6V7.3a1.9 1.9 0 0 1 1.2-1.8z",
    "M12 7.6v4.1",
    "!c 12 15.2 1",
  ],
  // Dome sides run straight before the half-round so the base rect and the
  // dome share a width; the two flashes sit on the 135°/45° rays.
  Siren: [
    "r 4.4 16.4 15.2 4 2",
    "M7.5 16.4v-4a4.5 4.5 0 0 1 9 0v4",
    "M7.1 7.5 5.7 6.1 M16.9 7.5 18.3 6.1",
  ],

  // ── controls ────────────────────────────────────────────────────────────
  // Six teeth. Tips are arcs on r 8.5, roots are arcs on r 6.2, and both run
  // clockwise about (12,12) - so the only straight parts are the flanks.
  Settings: [
    "M20.3 10.1A8.5 8.5 0 0 1 20.3 13.9L17.8 14.3A6.2 6.2 0 0 1 16.9 15.8L17.8 18.2A8.5 8.5 0 0 1 14.5 20.1L12.9 18.1A6.2 6.2 0 0 1 11.1 18.1L9.5 20.1A8.5 8.5 0 0 1 6.2 18.2L7.1 15.8A6.2 6.2 0 0 1 6.3 14.3L3.7 13.9A8.5 8.5 0 0 1 3.7 10.1L6.3 9.7A6.2 6.2 0 0 1 7.1 8.2L6.2 5.8A8.5 8.5 0 0 1 9.5 3.9L11.1 5.9A6.2 6.2 0 0 1 12.9 5.9L14.5 3.9A8.5 8.5 0 0 1 17.8 5.8L16.9 8.2A6.2 6.2 0 0 1 17.8 9.7L20.3 10.1z",
    "c 12 12 3",
  ],
  Settings2: [
    "M3.6 6.2h16.8 M3.6 12h16.8 M3.6 17.8h16.8",
    "c 8.4 6.2 1.9",
    "c 15.4 12 1.9",
    "c 10.4 17.8 1.9",
  ],
  SlidersHorizontal: ["M3.6 8.5h16.8 M3.6 15.5h16.8", "c 9 8.5 1.9", "c 15 15.5 1.9"],

  // ── weather / energy ────────────────────────────────────────────────────
  Waves: [
    "M3.9 7.2q2.7-2.2 5.4 0t5.4 0t5.4 0",
    "M3.9 12q2.7-2.2 5.4 0t5.4 0t5.4 0",
    "M3.9 16.8q2.7-2.2 5.4 0t5.4 0t5.4 0",
  ],
  // Curls are 270° of an r 2.3 circle; the middle gust is left plain so the
  // three lines never crowd below the 2.5px gap.
  Wind: [
    "M3.4 7h14a2.3 2.3 0 1 1-2.3-2.3",
    "M3.4 12h14.6",
    "M3.4 17h15.4a2.3 2.3 0 1 0-2.3 2.3",
  ],
  // Every one of the bolt's six vertices is an arc - the two apexes at r 0.9,
  // the four shoulders at r 1.15.
  Zap: [
    "M5.6 13.6a1.15 1.15 0 0 1-0.9-1.8l8.6-8.9a0.9 0.9 0 0 1 1.5 0.8l-1.4 4.8a1.15 1.15 0 0 0 1.1 1.4h3.9a1.15 1.15 0 0 1 0.9 1.8l-8.6 8.9a0.9 0.9 0 0 1-1.5-0.8l1.4-4.8a1.15 1.15 0 0 0-1.1-1.4z",
  ],

  // ── craft / personality (these replace emoji in demo data) ──────────────
  // Palette: a half-circle left edge, a thumb notch at the lower right, then a
  // single cubic back across the top. Two pips only, per the set's pip budget.
  Palette: [
    "M12 3.4a8.6 8.6 0 0 0 0 17.2 2.4 2.4 0 0 0 2.4-2.4c0-0.6-0.2-1.2-0.7-1.6a2.4 2.4 0 0 1 1.7-4.1h2a4.3 4.3 0 0 0 4.3-4.3C21.7 5.4 17.4 3.4 12 3.4z",
    "!c 7.4 11.2 1.15",
    "!c 10.8 7.4 1.15",
  ],
  Paperclip: [
    "M20.2 11.6 12.2 19.6a5.1 5.1 0 0 1-7.1-7.1l8.6-8.6a3.45 3.45 0 0 1 4.8 4.8l-8.5 8.5a1.75 1.75 0 0 1-2.4-2.4l7.9-7.9",
  ],
  FlaskConical: [
    "M9.7 3v6.4a2.2 2.2 0 0 1-0.3 1.2l-4.7 7.6a2.3 2.3 0 0 0 2 3.2h10.6a2.3 2.3 0 0 0 2-3.2l-4.7-7.6a2.2 2.2 0 0 1-0.3-1.2V3",
    "M9.7 3h4.6",
    "M7.6 15.4h8.8",
  ],
  // Head is a quarter of an r 9 circle centred at the handle's low end, so the
  // pick curves away from the shaft instead of meeting it in a V.
  Pickaxe: ["M9.6 5.4a9 9 0 0 1 9 9", "M16.1 7.9 4.6 19.4", "M12.7 9.1 14.9 11.3"],
  // Each leaf is two cubics closing on themselves - a lens, not a teardrop, so
  // the leaf tip is a cusp of two curves rather than a spike.
  Sprout: [
    "M12 21.2v-9.6",
    "M12 16C12 12 8.6 9.2 4.4 9.2c0 4 3.4 6.8 7.6 6.8z",
    "M12 11.6C12 8 15 5.2 18.6 5.2c0 3.6-3 6.4-6.6 6.4z",
  ],
  // Panels wide and flat, dish a cup rather than a dome. At near-square panels
  // and a domed dish this read as three dots in a row at avatar size - which
  // is the size it is actually used at.
  // A tall body between two wide ribbed arrays. Three similar rounded rects in
  // a row read as a molecule at any size; the body has to be the odd one out.
  // rx 1.6 here rather than the set's 2 - on a 4.4-wide bar, rx 2 is a stadium.
  Satellite: [
    "r 9.8 8.6 4.4 8 1.6",
    "r 2.4 10 6 5.2 1.6",
    "r 15.6 10 6 5.2 1.6",
    "M5.4 10v5.2M18.6 10v5.2M12 8.6V5.4M9.6 5.4h4.8",
  ],
};
