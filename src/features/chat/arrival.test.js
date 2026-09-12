import { beforeEach, describe, expect, it } from "vitest";
import { isArrival, resetArrivals } from "@/features/chat/arrival";

const T0 = 1_000_000;

beforeEach(() => resetArrivals(T0));

describe("deciding whether a message arrived or was already there", () => {
  it("lets a message made while watching arrive", () => {
    expect(isArrival("m1", { at: T0 + 50 })).toBe(true);
  });

  it("does not let a restored transcript arrive", () => {
    // Two hundred messages sliding up on open is the whole bug this exists
    // to prevent. Everything made before the window was watching was already
    // there when the person looked.
    for (let i = 0; i < 200; i += 1) {
      expect(isArrival(`old-${i}`, { at: T0 - 60_000 - i })).toBe(false);
    }
  });

  it("does not let an older message revealed by a click arrive", () => {
    // The transcript window mounts these for the first time long after the
    // conversation opened; they are old, and the timestamp says so.
    expect(isArrival("m-old", { at: T0 - 3_600_000 })).toBe(false);
  });

  it("arrives once, not on every remount", () => {
    // Switching to another thread and back mounts the same message again.
    expect(isArrival("m1", { at: T0 + 10 })).toBe(true);
    expect(isArrival("m1", { at: T0 + 10 })).toBe(false);
  });

  it("treats a thing that is still happening as arriving", () => {
    // A tool call has no timestamp; a card that mounts while its call is
    // running is one appearing in the middle of a reply.
    expect(isArrival("call-1", { live: true })).toBe(true);
  });

  it("treats a finished call being mounted as history", () => {
    expect(isArrival("call-2", { live: false })).toBe(false);
    expect(isArrival("call-3", {})).toBe(false);
  });

  it("remembers a live call so it does not arrive twice", () => {
    expect(isArrival("call-1", { live: true })).toBe(true);
    expect(isArrival("call-1", { live: true })).toBe(false);
  });

  it("remembers something it decided was old, too", () => {
    // Or a message that was old at mount could become "new" later on some
    // other evidence. Once seen is once seen.
    expect(isArrival("m1", { at: T0 - 1 })).toBe(false);
    expect(isArrival("m1", { live: true })).toBe(false);
  });

  it("decides on the evidence alone when there is no id", () => {
    expect(isArrival(null, { live: true })).toBe(true);
    expect(isArrival(null, { live: true })).toBe(true);
    expect(isArrival(undefined, { at: T0 - 1 })).toBe(false);
  });

  it("shrugs at a timestamp that is not one", () => {
    expect(isArrival("m1", { at: undefined })).toBe(false);
    expect(isArrival("m2", { at: "soon" })).toBe(false);
    expect(isArrival("m3", { at: null })).toBe(false);
  });

  it("does not grow without limit", () => {
    // Keyed on every message and tool call ever drawn, so it has to forget.
    for (let i = 0; i < 5000; i += 1) isArrival(`m-${i}`, { at: T0 + 1 });
    // The earliest has been forgotten and would arrive again; the latest is
    // still known.
    expect(isArrival("m-0", { at: T0 + 1 })).toBe(true);
    expect(isArrival("m-4999", { at: T0 + 1 })).toBe(false);
  });
});
