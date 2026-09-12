/**
 * Tool-surface glyphs - the few shapes a tool call and a permission ask need
 * that the rest of the set does not.
 *
 * Two of these are deliberately derived rather than drawn fresh:
 *
 *   · ShieldQuestion reuses the exact shield silhouette from `glyphs.status`,
 *     so the safety family (ShieldCheck, ShieldAlert, ShieldQuestion) reads as
 *     one object with three faces. A permission prompt sits a few pixels from
 *     a denied call in the transcript; a second shield outline would show.
 *   · ListChecks borrows core `CircleCheck`'s elbow-with-a-real-arc tick, at
 *     the same 0.85 scale ShieldCheck uses, so the checklist's ticks and the
 *     shield's tick are the same tick.
 *
 * Diff is the odd one out and is drawn from scratch: a frame with a plus above
 * and a bar below. It has to survive at 14px in a hunk header, which rules out
 * anything with two sets of chevrons.
 */
export const GLYPHS_TOOLS = {
  // Two ticks against three rules. The rules sit at 7.6 / 12 / 16.4 and the
  // ticks land on the outer two, so the middle rule reads as the unticked one
  // rather than as a stray line.
  ListChecks: [
    "M3.4 7.4 5.1 9.1a0.9 0.9 0 0 0 1.4-0.1L9 5.8",
    "M3.4 15.8 5.1 17.5a0.9 0.9 0 0 0 1.4-0.1L9 14.2",
    "M12.4 7.6h8.2 M12.4 12h8.2 M12.4 16.4h8.2",
  ],

  // rx 4 on a 17.6 square, matching the set's card-shaped frames. The plus arms
  // are 4.4 long against a 7.2 bar, so the bar always reads as the wider mark.
  Diff: [
    "r 3.2 3.2 17.6 17.6 4",
    "M12 7.4v4.4 M9.8 9.6h4.4",
    "M8.4 16.2h7.2",
  ],

  // THE shield, unchanged from glyphs.status, plus a hook that stops short of
  // the pip - a question mark whose tail touches its dot closes up at 16px.
  ShieldQuestion: [
    "M11.1 2.8a2.2 2.2 0 0 1 1.8 0l6.5 2.7a1.9 1.9 0 0 1 1.2 1.8v4.3c0 4.4-2.9 7.6-7.5 9.6a2.4 2.4 0 0 1-2.2 0c-4.6-2-7.5-5.2-7.5-9.6V7.3a1.9 1.9 0 0 1 1.2-1.8z",
    "M10 9.5a2 2 0 1 1 2.7 1.9c-0.5 0.2-0.7 0.6-0.7 1.1v0.5",
    "!c 12 15.6 1",
  ],
};
