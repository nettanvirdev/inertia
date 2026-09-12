import { describe, expect, it } from "vitest";
import { isFeatureOn, visibleItems } from "./features.js";

const NAV = [
  { id: "agents" },
  { id: "memory", needs: "memory" },
  { id: "activity" },
];

describe("which parts of the app exist", () => {
  it("keeps everything when nothing has been switched off", () => {
    expect(visibleItems(NAV, {}).map((i) => i.id)).toEqual(["agents", "memory", "activity"]);
  });

  it("removes a screen whose feature is off, leaving no gap", () => {
    expect(visibleItems(NAV, { memory: false }).map((i) => i.id)).toEqual(["agents", "activity"]);
  });

  it("treats a preference nobody has set as on", () => {
    // A workspace saved before the feature existed should get the feature,
    // not a blank screen where it used to be.
    expect(isFeatureOn(undefined, "memory")).toBe(true);
    expect(isFeatureOn({}, "memory")).toBe(true);
    expect(isFeatureOn({ memory: null }, "memory")).toBe(true);
  });

  it("only counts an explicit false as off", () => {
    expect(isFeatureOn({ memory: false }, "memory")).toBe(false);
    expect(isFeatureOn({ memory: true }, "memory")).toBe(true);
  });

  it("leaves an entry alone when it names a feature nobody defined", () => {
    expect(visibleItems([{ id: "x", needs: "invented" }], {})).toHaveLength(1);
  });
});
