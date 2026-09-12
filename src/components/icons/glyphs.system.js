/**
 * System glyphs - machines, infrastructure and tools.
 *
 * Two internal motifs hold this batch together:
 *
 *   · THE SCREEN. Every screen-shaped glyph (Monitor, MonitorPlay, Laptop,
 *     Container and both terminal boxes) uses the same ~1.45:1 rect at rx 3.5.
 *     Monitor adds a stand, Laptop a wide base, Container two ribs.
 *   · THE SLAB. Every stacked-machine glyph (Server, ServerCog, HardDrive,
 *     Blocks) is built from a 7-high slab with a 2.5 gap, so the rack bands,
 *     the drive body and the stepped blocks all sit on one rhythm.
 *
 * Reused from core: the Play triangle inside MonitorPlay, the arrowhead arc in
 * Repeat, the chevron elbow in all three terminals, and the Square box radius.
 */
export const GLYPHS_SYSTEM = {
  // Pulse, not zig-zag: every peak and trough turns through a real arc.
  Activity: ["M2.5 12h3.6l2.4-6a1.1 1.1 0 0 1 2 .1l3.4 11a1.1 1.1 0 0 0 2 .1l2-5.2h3.6"],

  // Three slabs on the shared 7-high / 2.5-gap rhythm, stepped into an L.
  Blocks: ["r 2.5 3.5 8 7 2.5", "r 13 3.5 8 7 2.5", "r 13 13 8 7 2.5"],

  Cloud: ["M17.3 19H9a6.8 6.8 0 1 1 6.5-8.8h1.8a4.4 4.4 0 1 1 0 8.8z"],
  // A lid band plus two ribs. Ribs alone read as Pause in a box at two, and
  // as SquareKanban at three - the band is what makes it a crate.
  Container: ["r 2.8 5.5 18.4 13 3.5", "M2.8 9.6h18.4", "M9.2 12.6v3.4M14.8 12.6v3.4"],

  // Pins capped at two per side - three merge into a smear at 16px.
  // A bigger die and shorter pins. The first cut had a small die ringed by
  // long pins, which collapsed into a sunburst at 16px.
  Cpu: [
    "r 5 5 14 14 3.5",
    "r 9 9 6 6 2.5",
    "M9.3 5V2.8M14.7 5V2.8M9.3 19v2.2M14.7 19v2.2M5 9.3H2.8M5 14.7H2.8M19 9.3h2.2M19 14.7h2.2",
  ],
  CreditCard: ["r 2.5 5 19 14 3.5", "M2.5 9.5h19", "M6.5 15h3.5"],

  // Rhombus with all four vertices arced - a raw diamond is four spikes.
  Diamond: [
    "M10.6 3.1a2 2 0 0 1 2.8 0l7.5 7.5a2 2 0 0 1 0 2.8l-7.5 7.5a2 2 0 0 1-2.8 0l-7.5-7.5a2 2 0 0 1 0-2.8z",
  ],
  GitBranch: ["c 6.5 18 2.6", "c 17.5 6 2.6", "M6.5 15.4V8.6a2.6 2.6 0 0 1 2.6-2.6h5.8"],

  // One meridian, one equator. A third interior line turns to mush at 16px.
  Globe: ["c 12 12 8.5", "M3.5 12h17", "M12 3.5a4.6 8.5 0 0 1 0 17a4.6 8.5 0 0 1 0-17z"],
  // Head square to the handle. Drawn at 45 it read as a marker pen, because
  // head and handle ended up on the same axis.
  Hammer: ["r 4.5 4.4 15 4.6 2.3", "M12 9v11.2"],
  HardDrive: ["r 2.5 8.5 19 7 3", "!c 6.5 12 1.2", "M10.5 12h7.5"],
  KeyRound: ["c 7.5 16.5 4", "M10.3 13.7 20 4", "M16 8 17.6 9.6", "M13.9 10.1 15.5 11.7"],
  Keyboard: ["r 2.5 5.5 19 13 3", "M6.2 10h1.5M11.2 10h1.5M16.2 10h1.5", "M8 14.5h8"],
  Laptop: ["r 4 3.8 16 11 3.5", "M2.8 18.2h18.4"],
  Monitor: ["r 2.5 3.5 19 13 3.5", "M12 16.5v3", "M8.5 19.5h7"],

  // The core Play head, scaled to ~0.45 and centred in the screen rect.
  MonitorPlay: [
    "r 2.5 3.5 19 13 3.5",
    "M8.8 7.4a1 1 0 0 1 1.5-.9l4.4 2.6a1 1 0 0 1 0 1.8l-4.4 2.6a1 1 0 0 1-1.5-.9z",
    "M12 16.5v3",
    "M8.5 19.5h7",
  ],
  Plug: [
    "M9 3.2v4M15 3.2v4",
    "M6.6 9.2a2 2 0 0 1 2-2h6.8a2 2 0 0 1 2 2v1.3a5.4 5.4 0 0 1-10.8 0z",
    "M12 15.9v5.1",
  ],
  PowerOff: ["M12 3.4v7.2", "M17.4 6.6a7.7 7.7 0 1 1-10.8 0"],

  // One outline: body, then a knob on the top, right and left edges.
  Puzzle: [
    "M4.5 12.5V7a2.5 2.5 0 0 1 2.5-2.5h3.3a2 2 0 1 1 3.4 0H17a2.5 2.5 0 0 1 2.5 2.5v3.3a2 2 0 1 0 0 3.4V17a2.5 2.5 0 0 1-2.5 2.5H7A2.5 2.5 0 0 1 4.5 17v-1.2a2 2 0 1 0 0-3.3z",
  ],

  // Rounded loop, both heads built from the core arrowhead arc.
  Repeat: [
    "M3.5 12.5V10a3 3 0 0 1 3-3h13.3",
    "M17 4.2 19.6 6.1a1.2 1.2 0 0 1 0 1.9L17 9.8",
    "M20.5 11.5V14a3 3 0 0 1-3 3H4.2",
    "M7 14.2 4.4 16.1a1.2 1.2 0 0 0 0 1.9L7 19.8",
  ],
  Rocket: [
    "M12 2.6c3.2 2.2 5 5.6 5 9.4v3.5a1.5 1.5 0 0 1-.6 1.2l-3.5 2.6a1.5 1.5 0 0 1-1.8 0l-3.5-2.6A1.5 1.5 0 0 1 7 15.5V12c0-3.8 1.8-7.2 5-9.4z",
    "c 12 10 1.9",
    "M7 14.5 4 17.4a1.5 1.5 0 0 0-.4 1V21M17 14.5l3 2.9a1.5 1.5 0 0 1 .4 1V21",
  ],
  Route: [
    "c 5.5 18.5 2.6",
    "c 18.5 5.5 2.6",
    "M8.1 18.5h6.9a3.4 3.4 0 0 0 0-6.8H9a3.4 3.4 0 0 1 0-6.8h6.9",
  ],
  // Pans are quadratics, so the cups curve instead of hanging as two V spikes.
  Scale: ["M12 6.8v12.4", "M8 19.2h8", "M5.5 6.8h13", "M2.5 9.8q3 4.6 6 0", "M15.5 9.8q3 4.6 6 0"],
  Server: ["r 2.5 3.5 19 7 3", "r 2.5 13 19 7 3", "!c 6 7 1.1", "!c 6 16.5 1.1"],
  ServerCog: [
    "r 2.5 3.5 19 7 3",
    "!c 6 7 1.1",
    "c 12 16.6 2.4",
    "M12 13.2v1M12 19v1M8.2 16.6h-1M15.8 16.6h1",
  ],
  // Prompt only - its twin TerminalSquare is the one that carries the caret.
  SquareTerminal: ["r 3.5 3.5 17 17 3.5", "M7.5 8.8 10.6 11.4a1.2 1.2 0 0 1 0 1.2L7.5 15.2"],
  Terminal: ["M5 7.6 9.6 11.4a1.4 1.4 0 0 1 0 1.2L5 16.4", "M13 16.4h6"],
  TerminalSquare: [
    "r 3.5 3.5 17 17 3.5",
    "M7.2 8.6 9.8 10.8a1.2 1.2 0 0 1 0 1.2L7.2 14.2",
    "M12.8 15.2h3.8",
  ],
  Workflow: [
    "r 3 3 7.5 7.5 2.5",
    "r 13.5 13.5 7.5 7.5 2.5",
    "M10.5 6.8h4.2a2.5 2.5 0 0 1 2.5 2.5v4.2",
  ],
  Wrench: [
    "M15 6.6a1 1 0 0 0 0 1.4l1.5 1.5a1 1 0 0 0 1.4 0l3.4-3.4a5.8 5.8 0 0 1-7.7 7.7l-6.5 6.5a2.1 2.1 0 0 1-3-3l6.5-6.5a5.8 5.8 0 0 1 7.7-7.7l-3.3 3.5z",
  ],
  Wifi: [
    "M2.6 9.1a13.5 13.5 0 0 1 18.8 0",
    "M6.1 12.6a8.5 8.5 0 0 1 11.8 0",
    "M9.5 16.1a5 5 0 0 1 5 0",
    "!c 12 19.6 1.3",
  ],
};
