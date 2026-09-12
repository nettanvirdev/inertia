/**
 * Science glyphs - the shapes a workspace full of researchers needs and the
 * rest of the set has no answer for.
 *
 * The app had a flask and a satellite and nothing else: a mathematician, a
 * biologist and a statistician all fell back to the same generic dot. These
 * are drawn to the same grid as everything else - 24x24, 2px margin, stroke
 * 1.5, nothing sharper than a 2px corner - so a physicist's atom beside a
 * folder icon in the sidebar reads as one family rather than as clip art.
 *
 * Two of them are built from arcs rather than circles, which is worth knowing
 * before editing: Atom and Orbit are ellipses turned off the axis, and an
 * ellipse cannot be rotated in this shorthand. Each is written as two half
 * ellipses whose endpoints are the ends of the major axis - the same sweep
 * both times, which is what makes the pair close into one continuous ring.
 */
export const GLYPHS_SCIENCE = {
  // The summation sign, on the same 5..19 vertical the set's other letterforms
  // use. The waist stops at 12.4 rather than the centre so the two diagonals
  // are the same length as the bars - a symmetric sigma reads as squashed.
  Sigma: ["M16.8 5H7.2l5.2 7-5.2 7h9.6"],

  // Two orbits at 45 and -45, and a nucleus with a real fill. The shells are
  // 9.4 by 4: any fatter and the crossings at the top and bottom close up into
  // a blob at 16px, any thinner and the glyph reads as a bow tie.
  Atom: [
    "M5.4 5.4a9.4 4 45 1 1 13.2 13.2 9.4 4 45 1 1-13.2-13.2",
    "M18.6 5.4a9.4 4 -45 1 1-13.2 13.2 9.4 4 -45 1 1 13.2-13.2",
    "!c 12 12 1.7",
  ],

  // Three atoms and the bonds between them. The bonds stop 2.1 short at each
  // end - the radius of the ring they are leaving - so no line is drawn
  // through a circle it is meant to be touching.
  Molecule: [
    "l 7.2 15.4 11 8.4",
    "l 13.2 8.3 16.6 13.3",
    "l 8.3 16.8 15.7 15.4",
    "c 6.2 17.2 2.1",
    "c 12 6.6 2.1",
    "c 17.8 15 2.1",
  ],

  // A body, the path around it, and something small going round. Tilted 25
  // degrees because a level ring reads as a plate seen edge-on; off the
  // horizontal it reads as an orbit.
  Orbit: [
    "M3.3 16.1a9.6 3.4 -25 1 1 17.4-8.2 9.6 3.4 -25 1 1-17.4 8.2",
    // 3 rather than 3.6: a body that nearly fills its own ring reads as an eye
    // with a lid, not as something being gone around.
    "c 12 12 3",
    "!c 19.9 10.5 1.5",
  ],

  // Two strands crossing twice, with the rungs where the strands are furthest
  // apart. A rung at the crossing would be a dot, and three rungs at this size
  // is a ladder rather than a helix.
  Dna: [
    "M7.5 3.2c0 4.6 9 5.6 9 8.8s-9 4.2-9 8.8",
    "M16.5 3.2c0 4.6-9 5.6-9 8.8s9 4.2 9 8.8",
    "l 9.5 7.3 14.5 7.3",
    "l 9.5 16.7 14.5 16.7",
  ],

  // An axis with a real corner radius, and three bars standing on it. The bars
  // are lines rather than rects: at 14px a filled bar with a 1.5 stroke is a
  // black rectangle, and a stroked one is a box with a hole in it.
  BarChart: [
    "M5 4.6v12.8a1.8 1.8 0 0 0 1.8 1.8H19.4",
    "l 9.6 19.2 9.6 14.4",
    "l 13.2 19.2 13.2 9.2",
    "l 16.8 19.2 16.8 12.2",
  ],
};
