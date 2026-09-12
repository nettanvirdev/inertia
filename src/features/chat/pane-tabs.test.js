import { describe, expect, it } from "vitest";
import {
  activeOf,
  addTab,
  adoptTab,
  closeTab,
  kindOfTab,
  labelFor,
  nextSerial,
  serialOf,
  tabId,
  threadOfTab,
} from "./pane-tabs.js";

const open = (count, kind = "web") => {
  let state = { tabs: [], active: null };
  for (let i = 0; i < count; i += 1) {
    state = addTab(state.tabs, { threadId: "t1", kind });
  }
  return state;
};

describe("tabId", () => {
  it("is unique across conversations and kinds", () => {
    expect(tabId("t1", "web", 1)).toBe("chat:t1:web:1");
    expect(tabId("t1", "web", 1)).not.toBe(tabId("t2", "web", 1));
    expect(tabId("t1", "web", 1)).not.toBe(tabId("t1", "sh", 1));
  });
});

describe("nextSerial", () => {
  it("counts from the highest ever used, not from the length", () => {
    // Open three, close the middle: the next one is 4, not 3 again.
    const three = open(3).tabs;
    const after = three.filter((tab) => tab.serial !== 2);
    expect(nextSerial(after)).toBe(4);
  });

  it("starts at one", () => {
    expect(nextSerial([])).toBe(1);
    expect(nextSerial(null)).toBe(1);
  });
});

describe("addTab", () => {
  it("puts the new tab in front, because that is why you opened it", () => {
    const state = addTab([], { threadId: "t1", kind: "web" });
    expect(state.active).toBe(state.tabs[0].id);
  });

  it("names a terminal a terminal", () => {
    expect(addTab([], { threadId: "t1", kind: "sh" }).opened.label).toBe("Terminal 1");
  });
});

describe("closeTab", () => {
  it("hands the front to the tab on the right", () => {
    const { tabs } = open(3);
    const [a, b, c] = tabs;
    const after = closeTab(tabs, b.id, b.id);
    expect(after.active).toBe(c.id);
    expect(after.tabs.map((t) => t.id)).toEqual([a.id, c.id]);
  });

  it("falls back to the left when the last tab closes", () => {
    const { tabs } = open(3);
    const after = closeTab(tabs, tabs[2].id, tabs[2].id);
    expect(after.active).toBe(tabs[1].id);
  });

  it("does not move the front when a background tab closes", () => {
    const { tabs } = open(3);
    const after = closeTab(tabs, tabs[0].id, tabs[2].id);
    expect(after.active).toBe(tabs[0].id);
  });

  it("leaves nothing in front when the only tab closes", () => {
    const { tabs, active } = open(1);
    const after = closeTab(tabs, active, tabs[0].id);
    expect(after.tabs).toEqual([]);
    expect(after.active).toBeNull();
  });

  it("ignores a tab that is not there", () => {
    const { tabs, active } = open(2);
    expect(closeTab(tabs, active, "nope")).toMatchObject({ active, closed: null });
  });
});

describe("activeOf", () => {
  it("keeps what was picked while it exists", () => {
    const { tabs } = open(2);
    expect(activeOf(tabs, tabs[1].id)).toBe(tabs[1].id);
  });

  it("falls back to the first when the picked one has gone", () => {
    const { tabs } = open(2);
    expect(activeOf(tabs, "gone")).toBe(tabs[0].id);
  });

  it("has no answer for an empty pane", () => {
    expect(activeOf([], "x")).toBeNull();
  });
});

describe("labelFor", () => {
  it("names a terminal after its folder", () => {
    expect(labelFor({ kind: "sh", serial: 1, cwd: "D:\\oss\\inertia" })).toBe("inertia");
    expect(labelFor({ kind: "sh", serial: 1, cwd: "/home/me/project/" })).toBe("project");
  });

  it("falls back to the number when the folder is not known yet", () => {
    expect(labelFor({ kind: "sh", serial: 2, cwd: "" })).toBe("Terminal 2");
  });

  it("names a browser tab after the page", () => {
    expect(labelFor({ kind: "web", serial: 1, title: "Example Domain", url: "https://example.com" })).toBe(
      "Example Domain"
    );
  });

  it("uses the host when the page has no title of its own", () => {
    expect(labelFor({ kind: "web", serial: 1, title: "", url: "https://example.com/a" })).toBe("example.com");
    // A title that is only the URL again is not a title.
    expect(
      labelFor({ kind: "web", serial: 1, title: "https://example.com/a", url: "https://example.com/a" })
    ).toBe("example.com");
  });

  it("falls back to the number for a tab with nothing open", () => {
    expect(labelFor({ kind: "web", serial: 3, title: "", url: "" })).toBe("Tab 3");
  });
});

describe("adopting a tab the agent reached first", () => {
  it("adds an id this list does not have", () => {
    // The agent navigates `chat:t1:web:1` before the pane has ever been
    // opened. If the pane minted its own tab instead, the person would watch
    // an empty one while the page loaded into a view they cannot see.
    const tabs = adoptTab([], "chat:t1:web:1", "web");
    expect(tabs).toEqual([{ id: "chat:t1:web:1", serial: 1, label: "Tab 1" }]);
  });

  it("keeps the number that is in the id", () => {
    const tabs = adoptTab([], "chat:t1:sh:7", "sh");
    expect(tabs[0]).toMatchObject({ serial: 7, label: "Terminal 7" });
  });

  it("does not put a pid where a tab number goes", () => {
    // A background process is revealed into the terminal pane, so the kind it
    // is adopted as is "sh" - and its id ends in a pid. "Terminal 20412" reads
    // as the twenty-thousandth terminal.
    const tabs = adoptTab([], "chat:t1:job:20412", "sh");
    expect(tabs[0].label).toBe("Process");
  });

  it("returns the same list when the tab is already there", () => {
    // Identity, not equality: a reveal of the tab you are looking at must not
    // re-render the pane.
    const tabs = [{ id: "chat:t1:web:1", serial: 1, label: "Tab 1" }];
    expect(adoptTab(tabs, "chat:t1:web:1", "web")).toBe(tabs);
  });

  it("does not disturb the tabs around it", () => {
    const tabs = [{ id: "chat:t1:web:1", serial: 1, label: "Renamed" }];
    const next = adoptTab(tabs, "chat:t1:web:4", "web");
    expect(next).toHaveLength(2);
    expect(next[0].label).toBe("Renamed");
  });

  it("declines an empty id rather than making a nameless tab", () => {
    const tabs = [{ id: "chat:t1:web:1", serial: 1, label: "Tab 1" }];
    expect(adoptTab(tabs, "", "web")).toBe(tabs);
    expect(adoptTab(tabs, null, "web")).toBe(tabs);
  });
});

describe("reading a tab id", () => {
  it("finds the number", () => {
    expect(serialOf("chat:t1:web:12")).toBe(12);
    expect(serialOf("nonsense")).toBe(0);
    expect(serialOf(null)).toBe(0);
  });

  it("finds the conversation, including one with colons in its id", () => {
    expect(threadOfTab("chat:t1:web:1")).toBe("t1");
    expect(threadOfTab("chat:a:b:c:sh:3")).toBe("a:b:c");
    expect(threadOfTab("chat:t1:other:1")).toBeNull();
    expect(threadOfTab("")).toBeNull();
  });
});

describe("a tab that mirrors something the app is running", () => {
  it("is told apart from a shell and a browser", () => {
    expect(kindOfTab("chat:t1:job:4242")).toBe("job");
    expect(kindOfTab("chat:t1:sh:2")).toBe("sh");
    expect(kindOfTab("chat:t1:web:1")).toBe("web");
    expect(kindOfTab("something-else")).toBeNull();
  });

  it("belongs to its conversation, keyed by pid rather than by number", () => {
    // The pid is what makes it findable again from the side that started it.
    expect(threadOfTab("chat:t1:job:4242")).toBe("t1");
  });

  it("is named after the command, not the pid", () => {
    // A tab saying "npm run dev" helps; one saying 4242 does not.
    expect(labelFor({ kind: "job", title: "npm run dev --workspace web" })).toBe("npm run dev");
    expect(labelFor({ kind: "job", title: "" })).toBe("Process");
  });

  it("cuts a long command rather than pushing the strip sideways", () => {
    const label = labelFor({ kind: "job", title: "node scripts/a-very-long-name.mjs --flag" });
    expect(label.length).toBeLessThanOrEqual(24);
  });
});
