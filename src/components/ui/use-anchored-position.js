import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

const MARGIN = 8; // viewport gutter every layer stays inside
const OPPOSITE = { top: "bottom", bottom: "top", left: "right", right: "left" };

const clamp = (v, min, max) => (max < min ? min : Math.min(Math.max(v, min), max));

/**
 * Fixed-position placement for a floating layer.
 * `anchorRef` may hold a real element or any object exposing
 * `getBoundingClientRect()` (a virtual anchor - see ContextMenu).
 *
 * `matchWidth` takes `true` (lock to the anchor's width), `"min"` (the anchor's
 * width is a floor and the sheet may grow past it) or `"anchor"` (the anchor's
 * width, unless that is too narrow to read an option in, in which case a floor
 * applies).
 *
 * "anchor" is what a Select wants. "min" was, and the result is a sheet wider
 * than the control it belongs to whenever one option in a long list happens to
 * be long - a panel that overhangs its own trigger on the right with nothing in
 * the overhang, which reads as a misplaced layer rather than as a wider list.
 * A label longer than the control truncates, which is what the control does
 * with it too.
 */

/**
 * Below this, a sheet cannot show an option, so it is allowed to be wider.
 *
 * Nine rem: narrow enough that no ordinary control - the smallest select in the
 * app is a nine-rem status filter - ever sees a sheet overhang it, wide enough
 * that a genuinely tiny trigger does not open a column of ellipses.
 */
const MIN_ANCHOR_WIDTH = 144;

export function useAnchoredPosition({
  anchorRef,
  open,
  side = "bottom",
  align = "start",
  offset = 6,
  matchWidth = false,
}) {
  const floatingRef = useRef(null);
  const [placedSide, setPlacedSide] = useState(side);
  const [style, setStyle] = useState({
    position: "fixed",
    top: 0,
    left: 0,
    // measured on the first layout pass; hidden (not unmounted) so it has a box
    visibility: "hidden",
  });

  const update = useCallback(() => {
    const anchor = anchorRef?.current;
    const floating = floatingRef.current;
    if (!anchor || !floating) return;

    const a = anchor.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    // offsetWidth/Height, not getBoundingClientRect: the open animation scales
    // the panel, and a bounding rect includes that transform - measuring
    // mid-animation would place the sheet against a 95%-size box.
    const fw = floating.offsetWidth;
    const fh = floating.offsetHeight;

    const minMode = matchWidth === "min";
    const anchorMode = matchWidth === "anchor";
    const anchorWidth = clamp(Math.max(a.width, MIN_ANCHOR_WIDTH), 0, vw - MARGIN * 2);
    const width = anchorMode
      ? anchorWidth
      : matchWidth && !minMode
        ? a.width
        : Math.max(fw, minMode ? a.width : 0);
    const height = fh;
    const vertical = side === "top" || side === "bottom";
    const need = vertical ? height : width;

    const room = {
      top: a.top - offset - MARGIN,
      bottom: vh - a.bottom - offset - MARGIN,
      left: a.left - offset - MARGIN,
      right: vw - a.right - offset - MARGIN,
    };

    // flip only when the preferred side cannot fit AND the opposite does better
    const flipped = OPPOSITE[side];
    const placed = room[side] < need && room[flipped] > room[side] ? flipped : side;

    let top;
    let left;

    if (placed === "bottom" || placed === "top") {
      top = placed === "bottom" ? a.bottom + offset : a.top - offset - height;
      left =
        align === "center"
          ? a.left + a.width / 2 - width / 2
          : align === "end"
            ? a.right - width
            : a.left;
    } else {
      left = placed === "right" ? a.right + offset : a.left - offset - width;
      top =
        align === "center"
          ? a.top + a.height / 2 - height / 2
          : align === "end"
            ? a.bottom - height
            : a.top;
    }

    left = clamp(left, MARGIN, vw - width - MARGIN);
    top = clamp(top, MARGIN, vh - height - MARGIN);

    // The growth point, in the panel's own pixels. Corner keywords ("left top")
    // are only right when the panel starts exactly at the trigger; the moment
    // it is centre-aligned, end-aligned, or clamped away from the viewport
    // edge, the corner is somewhere the pointer never was and the sheet reads
    // as a separate window fading in beside the control. Projecting the
    // trigger's centre onto the panel box makes it grow out of the click.
    const onVerticalAxis = placed === "top" || placed === "bottom";
    const originX = onVerticalAxis
      ? clamp(Math.round(a.left + a.width / 2 - left), 0, Math.round(width))
      : placed === "right"
        ? 0
        : Math.round(width);
    const originY = onVerticalAxis
      ? placed === "bottom"
        ? 0
        : Math.round(height)
      : clamp(Math.round(a.top + a.height / 2 - top), 0, Math.round(height));

    // A few pixels of travel away from the trigger, on the placement axis. The
    // scale alone reads as "appear"; the nudge reads as "come out of there".
    const slide = onVerticalAxis
      ? { x: "0px", y: placed === "bottom" ? "-4px" : "4px" }
      : { x: placed === "right" ? "-4px" : "4px", y: "0px" };

    setPlacedSide(placed);
    setStyle({
      position: "fixed",
      top: `${Math.round(top)}px`,
      left: `${Math.round(left)}px`,
      // so the sheet grows out of its trigger rather than out of nowhere
      transformOrigin: `${originX}px ${originY}px`,
      "--overlay-in-x": slide.x,
      "--overlay-in-y": slide.y,
      maxHeight: `${Math.max(
        96,
        Math.round(placed === "top" || placed === "bottom" ? room[placed] : vh - MARGIN * 2)
      )}px`,
      ...(matchWidth
        ? minMode
          ? { minWidth: `${Math.round(a.width)}px` }
          : { width: `${Math.round(anchorMode ? anchorWidth : a.width)}px` }
        : {}),
      visibility: "visible",
    });
  }, [anchorRef, side, align, offset, matchWidth]);

  useLayoutEffect(() => {
    if (!open) {
      setStyle((s) => (s.visibility === "hidden" ? s : { ...s, visibility: "hidden" }));
      return;
    }
    update();
  }, [open, update]);

  useEffect(() => {
    if (!open) return;

    const onChange = () => update();
    // capture:true so scrolling inside any ancestor pane repositions too
    window.addEventListener("scroll", onChange, true);
    window.addEventListener("resize", onChange);

    let ro;
    if (typeof ResizeObserver !== "undefined") {
      ro = new ResizeObserver(onChange);
      if (floatingRef.current) ro.observe(floatingRef.current);
      const anchor = anchorRef?.current;
      if (anchor instanceof Element) ro.observe(anchor);
    }

    return () => {
      window.removeEventListener("scroll", onChange, true);
      window.removeEventListener("resize", onChange);
      ro?.disconnect();
    };
  }, [open, update, anchorRef]);

  return { style, placedSide, floatingRef, update };
}
