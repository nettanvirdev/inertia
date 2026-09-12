import * as React from "react";

/**
 * Which number each footnote is written under, for one message.
 *
 * Numbering cannot live in the inline parser: `[^ref]` can appear in the first
 * paragraph and its text in the last block, and the parser sees one paragraph
 * at a time. So the markdown root collects the notes it was given, numbers them
 * in the order they were written, and puts the map here - which is the only
 * thing a reference in any paragraph needs to render.
 *
 * Its own file so the renderer and the root can both import it without either
 * importing the other.
 */
export const FootnoteNumbers = React.createContext(null);
