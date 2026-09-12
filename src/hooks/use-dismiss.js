import { useEffect } from "react";

/** Close a floating layer on outside pointerdown or Escape. */
export function useDismiss(refs, open, onClose) {
  useEffect(() => {
    if (!open) return;
    const list = Array.isArray(refs) ? refs : [refs];

    const onPointerDown = (e) => {
      if (list.some((r) => r?.current?.contains(e.target))) return;
      onClose();
    };
    const onKeyDown = (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onClose, refs]);
}
