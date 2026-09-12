/**
 * People, chat and marks.
 *
 * Three motifs are fixed here and repeated without variation:
 *
 *   · The head. Every User* glyph uses `c <cx> 8.2 3.2` - one radius, one
 *     baseline - and every yoke of shoulders is a 4.8-radius shoulder arc
 *     rising to y=14, so a User and a UserPlus sit on the same body.
 *   · The bubble. Every Message glyph is the same silhouette: a 4-radius
 *     box (the Square's 0.26 corner ratio, scaled) with one tail - a diagonal
 *     dropping from the bottom-left, its tip blunted with a quadratic rather
 *     than left as a spike.
 *   · The plus. UserPlus, MailPlus and MessageSquarePlus carry the core Plus
 *     verbatim - equal arms, centred, only the scale changes.
 *
 * Pin is a single closed teardrop on purpose: it is rendered with `fill-current`
 * from CSS, so it has to survive being painted solid. A pushpin's needle would
 * become two parallel strokes 1.2px apart when outlined, which merges to a blob
 * at 16px - the marker silhouette is the shape that reads both ways.
 */
export const GLYPHS_PEOPLE = {
  // The app's most-used glyph. A wide, very round head (rx 4.5 on a 12-tall
  // box), eyes as the set's only paired pips, and one short antenna.
  Agent: [
    "r 3.5 8 17 12 4.5",
    "M12 5.5v2.5",
    "!c 12 4.2 1.3",
    "!c 9 14 1.2",
    "!c 15 14 1.2",
  ],

  // Two mirrored lobes; the `z` of each half draws the central fissure, so the
  // whole glyph is two strokes and no separate divider line.
  Brain: [
    "M12 5.4a3.1 3.1 0 0 0-5.5 1.9 3 3 0 0 0-1.9 4.7 3.1 3.1 0 0 0 1.7 5.1 3.2 3.2 0 0 0 5.7 1.5z",
    "M12 5.4a3.1 3.1 0 0 1 5.5 1.9 3 3 0 0 1 1.9 4.7 3.1 3.1 0 0 1-1.7 5.1 3.2 3.2 0 0 1-5.7 1.5z",
  ],

  Contact: ["r 3 3 18 18 4.5", "c 12 9 3.2", "M6.6 18.4a5.9 5.9 0 0 1 10.8 0"],

  Inbox: [
    "M3.4 13.5v3.6a3 3 0 0 0 3 3h11.2a3 3 0 0 0 3-3v-3.6l-2.5-6.6a3 3 0 0 0-2.8-1.9H8.7a3 3 0 0 0-2.8 1.9z",
    "M3.4 13.5h4.2l1.1 2a1.8 1.8 0 0 0 1.6.9h3.4a1.8 1.8 0 0 0 1.6-.9l1.1-2h4.2",
  ],

  Mail: ["r 2.8 4.8 18.4 14.4 4", "M3.6 8.4 10.5 13a2.7 2.7 0 0 0 3 0l6.9-4.6"],
  MailPlus: [
    "M18.4 12.2V8.4a3.6 3.6 0 0 0-3.6-3.6H6A3.6 3.6 0 0 0 2.4 8.4v7.2A3.6 3.6 0 0 0 6 19.2h7.4",
    "M3 9.2 8.9 13.2a2.5 2.5 0 0 0 2.8 0l6-4",
    "M19 15.2v5.6",
    "M16.2 18h5.6",
  ],

  // THE round bubble: an 8.3 circle on (12,11.5) opened between 115° and 145°
  // for the tail, whose tip is blunted with a quadratic.
  MessageCircle: [
    "M20.3 11.5a8.3 8.3 0 0 1-11.8 7.5l-3.6 1.6q-1.3.6-.9-.7l1.2-3.6a8.3 8.3 0 1 1 15.1-4.8z",
  ],
  MessageCircleQuestion: [
    "M20.3 11.5a8.3 8.3 0 0 1-11.8 7.5l-3.6 1.6q-1.3.6-.9-.7l1.2-3.6a8.3 8.3 0 1 1 15.1-4.8z",
    "M9.8 9.4a2.3 2.3 0 0 1 4.5.7c0 1.5-2.3 2.3-2.3 2.3",
    "!c 12 15.9 1.05",
  ],

  // THE square bubble. Reused verbatim by MessageSquarePlus and, scaled, by the
  // front bubble of MessagesSquare.
  MessageSquare: [
    "M8 4.5h8.5a4 4 0 0 1 4 4v4.5a4 4 0 0 1-4 4H9.4l-4.2 3.5q-1.2 1-1.2-.6V8.5a4 4 0 0 1 4-4z",
  ],
  MessageSquarePlus: [
    "M8 4.5h8.5a4 4 0 0 1 4 4v4.5a4 4 0 0 1-4 4H9.4l-4.2 3.5q-1.2 1-1.2-.6V8.5a4 4 0 0 1 4-4z",
    "M12.2 7.5v6.4",
    "M9 10.7h6.4",
  ],
  MessagesSquare: [
    "M7 8.5h5.5a3.5 3.5 0 0 1 3.5 3.5v3a3.5 3.5 0 0 1-3.5 3.5H7.6l-2.9 2.3q-1.2.9-1.2-.6V12a3.5 3.5 0 0 1 3.5-3.5z",
    "M7.5 8.5V7A3.5 3.5 0 0 1 11 3.5h6A3.5 3.5 0 0 1 20.5 7v3.5A3.5 3.5 0 0 1 17 14h-1",
  ],

  // Capsule, cradle, stem and foot. The foot bar is what stops the glyph
  // reading as a lollipop at 14px, where the cradle's arc is only a few
  // pixels deep and the stem has nothing to sit on.
  Mic: [
    "r 9 2 6 12 3",
    "M5 10v1a7 7 0 0 0 14 0v-1",
    "M12 18v4",
    "M9 22h6",
  ],

  // The handset, drawn as one continuous sweep rather than the usual two ear
  // pieces joined by a bar. At 14px a two-piece handset loses its middle and
  // reads as a comma; a single stroke that widens into a cradle at each end
  // keeps the silhouette whether it is 24px in a toolbar or 12px in a menu.
  // The corner radii are the set's standard 1.5 blunting, applied at the two
  // places the sweep turns back on itself.
  Phone: [
    "M8.2 4.4 5.5 5.6a2.2 2.2 0 0 0-1.3 2.4 15.5 15.5 0 0 0 11.8 11.8 2.2 2.2 0 0 0 2.4-1.3l1.2-2.7a1.4 1.4 0 0 0-.7-1.8l-3-1.3a1.4 1.4 0 0 0-1.6.4l-1 1.2a11.5 11.5 0 0 1-4.5-4.5l1.2-1a1.4 1.4 0 0 0 .4-1.6l-1.3-3a1.4 1.4 0 0 0-1.8-.7z",
  ],

  // Four apexes, four fillets: the nose (r 1.2), the tail point (r 1.3), the
  // left wingtip (r 1.2) and the notch between them (r 1, curving the other way).
  Send: [
    "M19.8 4a1.2 1.2 0 0 1 .7.7L12.4 19.5a1.3 1.3 0 0 1-1-.1L9.7 14.5a1 1 0 0 0-1.2-1.2L3.7 11.8a1.2 1.2 0 0 1 0-.9z",
  ],

  // The composer's send. A SOLID arrow, not the outlined paper plane above:
  // it sits inside a filled disc where a 1.5 stroke would be swallowed, and a
  // plain "up" reads as submit where a plane reads as mail.
  SendArrow: [
    "!M12 21a1.125 1.125 0 0 1-1.125-1.125V6.84L6.045 11.67a1.125 1.125 0 0 1-1.59-1.59l6.75-6.75a1.125 1.125 0 0 1 1.59 0l6.75 6.75a1.125 1.125 0 0 1-1.59 1.59L13.125 6.84v13.035A1.125 1.125 0 0 1 12 21Z",
  ],

  ThumbsUp: [
    "r 2.5 10.9 4.2 9 2.1",
    "M6.7 12.4h2a2 2 0 0 0 1.7-1l2.4-4.2a2.3 2.3 0 0 1 4.3 1.3l-.5 3h2.6a2.2 2.2 0 0 1 2 2.6l-.9 3.9a2.5 2.5 0 0 1-2.4 1.9H6.7",
  ],
  ThumbsDown: [
    "r 2.5 4.1 4.2 9 2.1",
    "M6.7 11.6h2a2 2 0 0 1 1.7 1l2.4 4.2a2.3 2.3 0 0 0 4.3-1.3l-.5-3h2.6a2.2 2.2 0 0 0 2-2.6l-.9-3.9a2.5 2.5 0 0 0-2.4-1.9H6.7",
  ],

  User: [
    "c 12 8.2 3.2",
    "M4.9 19.8v-1a4.8 4.8 0 0 1 4.8-4.8h4.6a4.8 4.8 0 0 1 4.8 4.8v1",
  ],
  UserPlus: [
    "c 9.6 8.2 3.2",
    "M2.6 19.8v-1a4.8 4.8 0 0 1 4.8-4.8h4.4a4.8 4.8 0 0 1 4.8 4.8v1",
    "M19.5 5.4v5.6",
    "M16.7 8.2h5.6",
  ],
  // Same head, but the yoke becomes one 7.5-radius cap - fuller, tighter crop.
  UserRound: ["c 12 8.2 3.2", "M3.8 20.6a8.4 8.4 0 0 1 16.4 0"],
  Users: [
    "c 8.6 8.2 3.2",
    "M2.2 20v-.8a4.8 4.8 0 0 1 4.8-4.8h3.2a4.8 4.8 0 0 1 4.8 4.8v.8",
    "M17.1 5.5a3.2 3.2 0 0 1 0 5.4",
    "M18.2 14.8a4.8 4.8 0 0 1 3.6 4.4v.8",
  ],

  // Five arcs, no miters: both outer spikes and the centre peak are blunted, and
  // both valleys are concave arcs, so nothing in the silhouette is a point.
  Crown: [
    "M4.9 18.6 4 8.7a1 1 0 0 1 1.6-.9l3 2.3a1.3 1.3 0 0 0 1.9-.4l1.4-3a1.3 1.3 0 0 1 2.2 0l1.4 3a1.3 1.3 0 0 0 1.9.4l3-2.3a1 1 0 0 1 1.6.9l-.9 9.9z",
  ],
  // Drawn from the bottom so the point is a fillet, not a vertex.
  Heart: [
    "M12.9 19.6 19.8 12.8a5 5 0 0 0-7.8-6.2 5 5 0 0 0-7.8 6.2l6.9 6.8a1.3 1.3 0 0 0 1.8 0z",
  ],
  // Four-point stars whose sides are quadratics pulled to the star's own centre.
  // Two stars, not three. The third was a 2.8-radius star, and below about
  // 20px a concave four-pointer that small stops resolving into a shape - it
  // renders as a smudge and drags the whole glyph down with it. The survivor
  // gets the space instead: a larger primary, one companion, real air between.
  Sparkles: [
    "M10 3.4Q10 10.4 3 10.4Q10 10.4 10 17.4Q10 10.4 17 10.4Q10 10.4 10 3.4z",
    "M18.3 13.6Q18.3 17.3 14.6 17.3Q18.3 17.3 18.3 21Q18.3 17.3 22 17.3Q18.3 17.3 18.3 13.6z",
  ],
  Lightbulb: ["M8.9 15.8a6.2 6.2 0 1 1 6.2 0", "M9.5 18.5h5", "M10.7 21.2h2.6"],
  // The mention mark. One closed inner circle and one outer arc that stops
  // short of closing - the gap at the lower right is the whole silhouette, and
  // an arc taken any further round reads as a pair of concentric rings.
  AtSign: ["c 12 12 3.7", "M15.7 12v1.7a2.6 2.6 0 0 0 5.2 0V12a9 9 0 1 0-3.6 7.2"],
  Binoculars: [
    "r 2.6 8.6 6 11.4 3",
    "r 15.4 8.6 6 11.4 3",
    "M8.6 13.2h6.8",
    "M3.6 8.6V6a2 2 0 0 1 2-2h.4a2 2 0 0 1 2 2v2.6M16.4 8.6V6a2 2 0 0 1 2-2h.4a2 2 0 0 1 2 2v2.6",
  ],
  // Fingers are four scalloped humps on one silhouette rather than four separate
  // strokes - separate fingers would sit ~0.6px apart and merge at 16px.
  Hand: [
    "M5.2 14.4V10.4a1.65 1.65 0 0 1 3.3 0V6.6a1.65 1.65 0 0 1 3.3 0V5a1.65 1.65 0 0 1 3.3 0v2.8a1.65 1.65 0 0 1 3.3 0v7a5.6 5.6 0 0 1-5.6 5.6h-1.6A5.6 5.6 0 0 1 5.2 14.8z",
  ],

  MousePointer2: [
    "M5.4 6 11.1 19.6a1.1 1.1 0 0 0 .8 0L13.6 14.7a1 1 0 0 1 1.2-1.2L19.4 11.8a1.1 1.1 0 0 0 0-.8L6 5.4a1.1 1.1 0 0 0-.6.6z",
  ],
  // Same cursor at 0.68 scale, tip parked on the click point the rays radiate from.
  MousePointerClick: [
    "M9.4 9.8 13.1 18.8a0.9 0.9 0 0 0 0.6 0L14.8 15.7a0.8 0.8 0 0 1 1-1L18.6 13.7a0.9 0.9 0 0 0 0-0.7L9.8 9.4a0.9 0.9 0 0 0-0.4 0.4z",
    "M9 3.6v2M3.6 9h2M5 5 6.4 6.4M13 5 11.6 6.4",
  ],

  // An actual pushpin, not a map marker. In this app "pinned" means a pinned
  // thread or memory, and a teardrop marker reads as a location every time.
  //
  // Both closed parts survive `fill-current`, which the memory cards paint on
  // to show pinned state; the needle is a line, so a fill leaves it a stroke.
  Pin: [
    "r 7.2 3.2 9.6 3.2 1.6",
    "M9.9 6.4 9.3 13.1a1.1 1.1 0 0 0 1.1 1.2h3.2a1.1 1.1 0 0 0 1.1-1.2l-.6-6.7z",
    "M12 14.3v6.4",
  ],
  // The same pin, opened at the cap so the core EyeOff slash passes through a
  // real gap rather than over an unbroken edge.
  PinOff: [
    "M9.9 6.4 9.3 13.1a1.1 1.1 0 0 0 1.1 1.2h3.2a1.1 1.1 0 0 0 1.1-1.2l-.3-3.4",
    "M8.8 3.2h8a1.6 1.6 0 0 1 1.6 1.6 1.6 1.6 0 0 1-1.6 1.6h-3.2",
    "M12 14.3v6.4",
    "M4.2 4.2 19.8 19.8",
  ],

  Pencil: [
    "M17.2 3.8a2.7 2.7 0 0 1 3.7 3.7L8 20.4l-4.4 1.1a0.9 0.9 0 0 1-1.1-1.1L4.3 16.4z",
    "M15.4 5.6 19.1 9.3",
  ],
  Trash2: [
    "M4.4 6.8h15.2",
    "M9.4 6.8V5.2a1.8 1.8 0 0 1 1.8-1.8h1.6a1.8 1.8 0 0 1 1.8 1.8v1.6",
    "M6.4 6.8v11.6a2.6 2.6 0 0 0 2.6 2.6h6a2.6 2.6 0 0 0 2.6-2.6V6.8",
    "M10.2 11v5.4M13.8 11v5.4",
  ],
  // A 45° block with a seam and a floor line - nothing vertical, nothing ribbed,
  // so it cannot be confused with Trash2 at a glance.
  Eraser: [
    "M10 4 17.2 11.2a2.4 2.4 0 0 1 0 3.4L14.6 17.2a2.4 2.4 0 0 1-3.4 0L4 10a2.4 2.4 0 0 1 0-3.4L6.6 4a2.4 2.4 0 0 1 3.4 0z",
    "M14.1 8.1 8.1 14.1",
    "M10 20.6h10.6",
  ],
};
