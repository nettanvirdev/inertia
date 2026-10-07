/**
 * Files, media and library glyphs.
 *
 * Three silhouettes are fixed here and reused verbatim across the batch:
 *
 *   · THE PAGE - a 14×18.4 sheet on x 5..19 with a 2.5 corner and a folded
 *     top-right ear. Every File* glyph is that page plus one interior idea.
 *   · THE FOLDER - one tab-and-body path on x 2.9..21.1. Every Folder* glyph
 *     is that body plus contents; FolderOpen swaps the body for a slanted
 *     front panel over the same back.
 *   · THE CONTAINER - "r 3.2 3.2 17.6 17.6 4". Frame, Image, Table,
 *     LayoutGrid, Rows3, SquareKanban, CheckSquare and Box all use that exact
 *     rect so the family reads as one box at 16px. Video's body is the only
 *     variant (a landscape 13.6×12.8) and keeps rx 4.
 *
 * Borrowed geometry: FileSearch is the core Search magnifier at 0.45 scale,
 * CheckSquare is the core Check elbow at 0.62 scale, PackagePlus carries the
 * core Plus at 0.47 scale. Layers / Package / Box are deliberately three
 * different constructions - a stack of plates, an isometric parcel, and a flat
 * crate with a lid seam - so they never collide at small sizes.
 */
export const GLYPHS_FILES = {
  AudioLines: ["M4.5 10.5v3", "M8.2 7.5v9", "M12 4.5v15", "M15.8 8.5v7", "M19.5 10.5v3"],

  // Spine drawn first; each leaf is one arc out of the spine and one back into
  // it, so both pages curl off the same centre line.
  BookOpen: [
    "M12 6.8v13.4",
    "M12 6.8a4.5 4.5 0 0 0-3.2-1.3H4.6a1.5 1.5 0 0 0-1.5 1.5v10.4a1.5 1.5 0 0 0 1.5 1.5h4.2a4.5 4.5 0 0 1 3.2 1.3",
    "M12 6.8a4.5 4.5 0 0 1 3.2-1.3h4.2a1.5 1.5 0 0 1 1.5 1.5v10.4a1.5 1.5 0 0 1-1.5 1.5h-4.2a4.5 4.5 0 0 0-3.2 1.3",
  ],
  Camera: [
    "M4.6 7.9h2.4a2 2 0 0 0 1.7-0.9l0.8-1.2a2 2 0 0 1 1.7-0.9h1.6a2 2 0 0 1 1.7 0.9l0.8 1.2a2 2 0 0 0 1.7 0.9h2.4a2.5 2.5 0 0 1 2.5 2.5v7.7a2.5 2.5 0 0 1-2.5 2.5H4.6a2.5 2.5 0 0 1-2.5-2.5v-7.7a2.5 2.5 0 0 1 2.5-2.5z",
    "c 12 14.2 3.4",
  ],

  // Both apexes are the core chevron arc (r 1.5) rather than a miter.
  Code: [
    "M9 6.8 3.6 10.9a1.5 1.5 0 0 0 0 2.2L9 17.2",
    "M15 6.8 20.4 10.9a1.5 1.5 0 0 1 0 2.2L15 17.2",
  ],
  // Deliberately a different idea from Code: a prompt caret and its input rule
  // inside the shared container, not a second pair of brackets.
  // Brackets plus a slash. The boxed prompt this used to be collided head-on
  // with SquareTerminal and TerminalSquare, which are also in the set.
  Code2: [
    "M8.6 7.4 4.7 11.2a1.2 1.2 0 0 0 0 1.6l3.9 3.8",
    "M15.4 7.4 19.3 11.2a1.2 1.2 0 0 1 0 1.6l-3.9 3.8",
    "M13.4 5.8 10.6 18.2",
  ],
  Database: [
    "M20 5.9c0 1.8 -3.6 3.2 -8 3.2c-4.4 0 -8 -1.4 -8 -3.2c0 -1.8 3.6 -3.2 8 -3.2c4.4 0 8 1.4 8 3.2z",
    "M4 5.9v12.2c0 1.8 3.6 3.2 8 3.2c4.4 0 8 -1.4 8 -3.2V5.9",
    "M4 12c0 1.8 3.6 3.2 8 3.2c4.4 0 8 -1.4 8 -3.2",
  ],

  // ── the page ──────────────────────────────────────────────────────────────
  File: [
    "M13.6 2.8H7.5A2.5 2.5 0 0 0 5 5.3v13.4A2.5 2.5 0 0 0 7.5 21.2h9A2.5 2.5 0 0 0 19 18.7V8.2z",
    "M13.6 2.9v3.3A2 2 0 0 0 15.6 8.2H19",
  ],
  FileJson: [
    "M13.6 2.8H7.5A2.5 2.5 0 0 0 5 5.3v13.4A2.5 2.5 0 0 0 7.5 21.2h9A2.5 2.5 0 0 0 19 18.7V8.2z",
    "M13.6 2.9v3.3A2 2 0 0 0 15.6 8.2H19",
    "M10.6 11.4a1.6 1.6 0 0 0-1.6 1.6v1a1 1 0 0 1-1 1 1 1 0 0 1 1 1v1a1.6 1.6 0 0 0 1.6 1.6",
    "M13.4 11.4a1.6 1.6 0 0 1 1.6 1.6v1a1 1 0 0 0 1 1 1 1 0 0 0-1 1v1a1.6 1.6 0 0 1-1.6 1.6",
  ],
  // Nib blunted with an r1 arc so the pencil tip is not a spike.
  FilePen: [
    "M13.6 2.8H7.5A2.5 2.5 0 0 0 5 5.3v13.4A2.5 2.5 0 0 0 7.5 21.2h9A2.5 2.5 0 0 0 19 18.7V8.2z",
    "M13.6 2.9v3.3A2 2 0 0 0 15.6 8.2H19",
    "M17.9 12.6a1.5 1.5 0 0 1 2.1 2.1l-4.6 4.6-2.4 0.6a1 1 0 0 1-1.2-1.2l0.6-2.4z",
  ],
  // Magnifier is the core Search (c 10.8 10.8 7.3 + its 45° handle) at 0.45.
  FileSearch: [
    "M13.6 2.8H7.5A2.5 2.5 0 0 0 5 5.3v13.4A2.5 2.5 0 0 0 7.5 21.2h9A2.5 2.5 0 0 0 19 18.7V8.2z",
    "M13.6 2.9v3.3A2 2 0 0 0 15.6 8.2H19",
    "c 12 14.5 3.3",
    "M14.3 16.8 16.4 18.9",
  ],
  FileText: [
    "M13.6 2.8H7.5A2.5 2.5 0 0 0 5 5.3v13.4A2.5 2.5 0 0 0 7.5 21.2h9A2.5 2.5 0 0 0 19 18.7V8.2z",
    "M13.6 2.9v3.3A2 2 0 0 0 15.6 8.2H19",
    "M8 12.4h8",
    "M8 15.4h8",
    "M8 18.4h8",
  ],

  // ── the folder ────────────────────────────────────────────────────────────
  Folder: [
    "M2.9 6.6a2.5 2.5 0 0 1 2.5-2.5h3.4a2 2 0 0 1 1.6 0.8l1.1 1.5a2 2 0 0 0 1.6 0.8h5.5a2.5 2.5 0 0 1 2.5 2.5v8.2a2.5 2.5 0 0 1-2.5 2.5H5.4a2.5 2.5 0 0 1-2.5-2.5z",
  ],
  FolderKanban: [
    "M2.9 6.6a2.5 2.5 0 0 1 2.5-2.5h3.4a2 2 0 0 1 1.6 0.8l1.1 1.5a2 2 0 0 0 1.6 0.8h5.5a2.5 2.5 0 0 1 2.5 2.5v8.2a2.5 2.5 0 0 1-2.5 2.5H5.4a2.5 2.5 0 0 1-2.5-2.5z",
    "M8.4 11.4v3.4",
    "M12 11.4v5.2",
    "M15.6 11.4v3.4",
  ],
  // The two folder verbs. Both keep the plain folder outline untouched and put
  // the mark low and centred in the body, where the tab does not crowd it -
  // a plus or a tick that had to dodge the tab would read as noise at 16px.
  FolderPlus: [
    "M2.9 6.6a2.5 2.5 0 0 1 2.5-2.5h3.4a2 2 0 0 1 1.6 0.8l1.1 1.5a2 2 0 0 0 1.6 0.8h5.5a2.5 2.5 0 0 1 2.5 2.5v8.2a2.5 2.5 0 0 1-2.5 2.5H5.4a2.5 2.5 0 0 1-2.5-2.5z",
    "M12 11.3v5",
    "M9.5 13.8h5",
  ],
  FolderCheck: [
    "M2.9 6.6a2.5 2.5 0 0 1 2.5-2.5h3.4a2 2 0 0 1 1.6 0.8l1.1 1.5a2 2 0 0 0 1.6 0.8h5.5a2.5 2.5 0 0 1 2.5 2.5v8.2a2.5 2.5 0 0 1-2.5 2.5H5.4a2.5 2.5 0 0 1-2.5-2.5z",
    "M9.1 13.9 11.2 16a1.2 1.2 0 0 0 1.8-0.1l2.9-3.6",
  ],
  // Back of the folder stays put; the front panel is a sheared rounded box so
  // the opening reads without a second outline.
  FolderOpen: [
    "M2.9 17.9V6.6a2.5 2.5 0 0 1 2.5-2.5h3.4a2 2 0 0 1 1.6 0.8l1.1 1.5a2 2 0 0 0 1.6 0.8h5.5a2.5 2.5 0 0 1 2.5 2.5v1.3",
    "M4.4 20.4h13.4a2 2 0 0 0 1.9-1.4l1.6-5.1a1.4 1.4 0 0 0-1.3-1.8H6.9a2 2 0 0 0-1.9 1.4l-1.6 5.1a1.4 1.4 0 0 0 1.3 1.8z",
  ],
  // One small folder and two branch runs out of a single trunk - the trunk
  // elbow carries the same r1.5 shoulder as the rest of the set.
  // The branches now end in real leaf nodes. Bare branch ends left the glyph
  // cramped and unreadable at 16px.
  FolderTree: [
    "M2.6 5.4a1.5 1.5 0 0 1 1.5-1.5h2.4l1.3 1.6h3.3a1.5 1.5 0 0 1 1.5 1.5v3a1.5 1.5 0 0 1-1.5 1.5H4.1a1.5 1.5 0 0 1-1.5-1.5z",
    "r 14.6 11.4 6.8 4 2",
    "r 14.6 16.6 6.8 4 2",
    "M6.4 11.5v7.1h8.2M6.4 13.4h8.2",
  ],

  // ── the container ─────────────────────────────────────────────────────────
  Frame: ["r 3.2 3.2 17.6 17.6 4", "r 7.2 7.2 9.6 9.6 2.8"],
  Image: [
    "r 3.2 3.2 17.6 17.6 4",
    "!c 8.8 8.8 1.3",
    "M3.9 18.4 9 13.3a1.7 1.7 0 0 1 2.4 0l5.1 5.1",
  ],
  Table: ["r 3.2 3.2 17.6 17.6 4", "M3.2 9.2h17.6", "M3.2 15h17.6", "M12 9.2v11.6"],
  LayoutGrid: ["r 3.2 3.2 17.6 17.6 4", "M3.2 12h17.6", "M12 3.2v17.6"],
  Rows3: ["r 3.2 3.2 17.6 17.6 4", "M3.2 9.1h17.6", "M3.2 14.9h17.6"],
  SquareKanban: ["r 3.2 3.2 17.6 17.6 4", "M8 7.6v5.2", "M12 7.6v8", "M16 7.6v3.4"],
  // The elbow is the core Check scaled 0.62 about (12,12).
  CheckSquare: ["r 3.2 3.2 17.6 17.6 4", "M8 12.2 10.1 14.5a0.9 0.9 0 0 0 1.4-0.1L16 9.3"],
  // An archive crate: lid off the body. As a square with a band it read as a
  // card, and sat too close to Frame and CreditCard.
  Box: [
    "M3.6 9.2h16.8v8.8a2.5 2.5 0 0 1-2.5 2.5H6.1a2.5 2.5 0 0 1-2.5-2.5z",
    "r 2.5 4.4 19 4.8 2.2",
    "M10.2 13.4h3.6",
  ],

  // ── stacks, parcels, shelves ──────────────────────────────────────────────
  Layers: [
    "M12.7 3.1 20.4 6.9a0.8 0.8 0 0 1 0 1.4l-7.7 3.8a1.6 1.6 0 0 1-1.4 0L3.6 8.3a0.8 0.8 0 0 1 0-1.4l7.7-3.8a1.6 1.6 0 0 1 1.4 0z",
    "M3.6 12.9 11.3 16.7a1.6 1.6 0 0 0 1.4 0l7.7-3.8",
    "M3.6 16.6 11.3 20.4a1.6 1.6 0 0 0 1.4 0l7.7-3.8",
  ],
  // Isometric parcel: six rounded shoulders and a Y seam, so it never reads as
  // the flat crate of Box.
  Package: [
    "M11.3 2.9a1.5 1.5 0 0 1 1.4 0l7.9 4.4a1.5 1.5 0 0 1 0.8 1.3v6.8a1.5 1.5 0 0 1-0.8 1.3l-7.9 4.4a1.5 1.5 0 0 1-1.4 0l-7.9-4.4a1.5 1.5 0 0 1-0.8-1.3V8.6a1.5 1.5 0 0 1 0.8-1.3z",
    "M3.1 7.9 12 12.9 20.9 7.9",
    "M12 12.9v8.2",
  ],
  // Right-bottom of the parcel is cut away to seat the core Plus at 0.47.
  PackagePlus: [
    "M20.9 12.2V8.6a1.5 1.5 0 0 0-0.8-1.3l-7.4-4.1a1.5 1.5 0 0 0-1.4 0L3.9 7.3a1.5 1.5 0 0 0-0.8 1.3v6.8a1.5 1.5 0 0 0 0.8 1.3l7.4 4.1a1.5 1.5 0 0 0 1.4 0l0.9-0.5",
    "M3.4 7.7 12 12.5 20.6 7.7",
    "M18 15.4v5.2",
    "M15.4 18h5.2",
  ],
  // Two upright spines and one leaning, standing on a shelf. As three stadium
  // rects it read as a bar chart in the sidebar - the lean is what says "books",
  // and this is a primary nav glyph, so it has to land without its label.
  Library: [
    "M3.4 20.4V7.8a1.4 1.4 0 0 1 1.4-1.4h.8a1.4 1.4 0 0 1 1.4 1.4v12.6",
    "M8.8 20.4V5.8a1.4 1.4 0 0 1 1.4-1.4h.8a1.4 1.4 0 0 1 1.4 1.4v14.6",
    "M16.2 20.4 14.6 9.6a1.4 1.4 0 0 1 1.2-1.6l1-.2a1.4 1.4 0 0 1 1.6 1.2l1.6 11.2",
    "M2.6 20.4h18.8",
  ],
  // Bullets are three subpaths of one element so the icon stays at four parts.
  List: ["M9 7.4h11", "M9 12h11", "M9 16.6h11", "M4 7.4h1.4M4 12h1.4M4 16.6h1.4"],
  ListTodo: ["r 3 4.4 6 6 2.2", "r 3 13.6 6 6 2.2", "M11.8 7.4h8.4", "M11.8 16.6h8.4"],
  NotebookText: ["r 5.2 3.2 15.6 17.6 3.6", "M9.2 3.2v17.6", "M12.4 9.6h5.4", "M12.4 14.4h5.4"],
  // Sheet plus one rolled top edge; the roll is a full half-circle so it reads
  // as a curl rather than a dog-ear.
  ScrollText: [
    "M7.4 3.4h10.2a2.5 2.5 0 0 1 2.5 2.5v11.2a2.5 2.5 0 0 1-2.5 2.5H7.4",
    "M7.4 3.4a2.2 2.2 0 0 0-2.2 2.2 2.2 2.2 0 0 0 2.2 2.2h2.4",
    "M7.4 7.8v11.8",
    "M11 10.6h6.4M11 14.2h6.4",
  ],
  Video: [
    "r 2.4 5.6 13.6 12.8 4",
    "M16 11.4 20.1 8.8a1 1 0 0 1 1.5 0.8v4.8a1 1 0 0 1-1.5 0.8L16 12.6z",
    "!c 6.4 9.4 1.1",
  ],
  // The two notches are half-circles cut into the long edges; the perforation
  // is one element with two subpaths.
  Ticket: [
    "M3 9.4V7.6a2.5 2.5 0 0 1 2.5-2.5h13A2.5 2.5 0 0 1 21 7.6v1.8a2.6 2.6 0 0 0 0 5.2v1.8a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 16.4v-1.8a2.6 2.6 0 0 0 0-5.2z",
    "M12 8v2.6M12 13.4v2.6",
  ],
};
