import { afterEach, describe, expect, it, vi } from "vitest";
import {
  isPaneDragging,
  onPaneDrag,
  resetPaneDrag,
  setPaneDragging,
} from "@/features/chat/pane-drag";

afterEach(() => resetPaneDrag());

describe("the drag flag the native panes watch", () => {
  it("starts down", () => {
    expect(isPaneDragging()).toBe(false);
  });

  it("tells everyone listening, once per change", () => {
    const heard = [];
    onPaneDrag((value) => heard.push(value));
    setPaneDragging(true);
    // The same value again is not a change, and a pane told to hide twice
    // would place itself twice for nothing.
    setPaneDragging(true);
    setPaneDragging(false);
    expect(heard).toEqual([true, false]);
  });

  it("coerces, because a pointer handler passes what it has", () => {
    const heard = [];
    onPaneDrag((value) => heard.push(value));
    setPaneDragging(1);
    setPaneDragging("yes");
    expect(heard).toEqual([true]);
    expect(isPaneDragging()).toBe(true);
  });

  it("keeps telling the others when one listener throws", () => {
    const after = vi.fn();
    onPaneDrag(() => {
      throw new Error("unmounted mid-notify");
    });
    onPaneDrag(after);
    setPaneDragging(true);
    expect(after).toHaveBeenCalledWith(true);
  });

  it("stops telling a pane that unsubscribed", () => {
    const gone = vi.fn();
    const off = onPaneDrag(gone);
    off();
    setPaneDragging(true);
    expect(gone).not.toHaveBeenCalled();
  });

  it("survives being handed something that is not a listener", () => {
    expect(() => onPaneDrag(null)()).not.toThrow();
  });
});
