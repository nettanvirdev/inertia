import { describe, expect, it } from "vitest";
import { activeTab, moveTab, orderTabs } from "./dock-tabs.js";

const sections = [{ id: "web" }, { id: "shell" }, { id: "thread" }];

describe("orderTabs", () => {
  it("follows the remembered order", () => {
    expect(orderTabs(sections, ["thread", "web", "shell"]).map((s) => s.id)).toEqual([
      "thread",
      "web",
      "shell",
    ]);
  });

  it("keeps a pane the stored order has never heard of", () => {
    expect(orderTabs(sections, ["thread"]).map((s) => s.id)).toEqual(["thread", "web", "shell"]);
  });

  it("ignores an id for a pane that no longer exists", () => {
    expect(orderTabs(sections, ["gone", "shell"]).map((s) => s.id)).toEqual([
      "shell",
      "web",
      "thread",
    ]);
  });

  it("has nothing to order when there is nothing", () => {
    expect(orderTabs([], ["a"])).toEqual([]);
    expect(orderTabs(sections, null).map((s) => s.id)).toEqual(["web", "shell", "thread"]);
  });
});

describe("activeTab", () => {
  it("keeps what was picked while it is still open", () => {
    expect(activeTab(sections, "shell")).toBe("shell");
  });

  it("hands the front to the first tab when the picked one has closed", () => {
    expect(activeTab([{ id: "web" }, { id: "thread" }], "shell")).toBe("web");
  });

  it("has no answer when nothing is open", () => {
    expect(activeTab([], "web")).toBeNull();
  });
});

describe("moveTab", () => {
  const order = ["a", "b", "c", "d"];

  it("moves one leftwards", () => {
    expect(moveTab(order, "d", "b")).toEqual(["a", "d", "b", "c"]);
  });

  it("moves one rightwards to where it looks like it will go", () => {
    expect(moveTab(order, "a", "c")).toEqual(["b", "a", "c", "d"]);
  });

  it("drops at the end when there is nothing to go before", () => {
    expect(moveTab(order, "a", null)).toEqual(["b", "c", "d", "a"]);
  });

  it("leaves the order alone when the target is not in it", () => {
    expect(moveTab(order, "a", "zzz")).toEqual(["b", "c", "d", "a"]);
  });
});
