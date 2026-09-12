/**
 * The measurements every managing view shares.
 *
 * Seven screens each hand-roll their own title bar, and measuring them showed
 * the rhythm was already right: 56px tall, a 14px medium title, an 11px count
 * beside it, controls pushed right. What had drifted was the gutter. Five views
 * sat at `px-4 sm:px-6` and three at `px-4`, so the title jumped eight pixels
 * left every time you opened Computers, Integrations or a chat and eight back
 * when you left. Nobody reports that; everybody feels it.
 *
 * These are strings rather than components on purpose. A `<ViewHeader>` would
 * have to swallow seven different arrangements of counts, filters, segmented
 * controls and menus, and the version of it that fits all seven is a component
 * with nine props - which is the same seven layouts again, just further away
 * from the screen they belong to. What actually needed to be one thing was the
 * number, so the number is the thing that is shared.
 */

/** The page gutter. Header, toolbar and body all take it, so a row lines up
 *  under the title above it. `view-gutter.test.js` fails if a view writes its
 *  own. */
export const GUTTER = "px-4 sm:px-6";

/**
 * One row rhythm, for every list in the app.
 *
 * There were four: `px-3` at a fixed 56px height, `px-2.5 py-1.5`,
 * `px-2.5 py-2`, and `px-3 py-2.5` with a permanent fill - three horizontal
 * insets, three gaps and two corner radii across four lists a person reads in
 * the same sitting.
 *
 * The negative margin is the part worth keeping. A row inside the page gutter
 * that also has padding of its own starts its text twelve pixels right of the
 * title above it: close enough to look like a mistake rather than a decision.
 * Pulling the row back out by exactly its own padding puts the text on the
 * title's edge and lets the hover fill bleed into the gutter, which is what
 * makes a hovered row read as a band across the pane rather than a lozenge
 * floating in the middle of one.
 */
export const ROW = "-mx-3 rounded-xl px-3 py-2.5";

/** The gap between a header's title, its count and its controls. */
export const HEADER_GAP = "gap-2.5";
