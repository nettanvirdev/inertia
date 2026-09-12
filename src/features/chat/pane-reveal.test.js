import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clearReveal,
  onRevealPane,
  pendingReveals,
  resetPaneReveal,
  revealPane,
} from "@/features/chat/pane-reveal";

afterEach(() => resetPaneReveal());

describe("bringing the pane the agent is using forward", () => {
  it("tells everyone which panel and which tab", () => {
    const heard = [];
    onRevealPane((detail) => heard.push(detail));
    revealPane({ dock: "web", tabId: "chat:t1:web:2", threadId: "t1" });
    expect(heard).toEqual([{ dock: "web", tabId: "chat:t1:web:2", threadId: "t1" }]);
  });

  it("carries the conversation, so another thread's panels stay put", () => {
    // Three components act on this, and none of them can tell whose tool ran.
    const heard = [];
    onRevealPane((detail) => heard.push(detail.threadId));
    revealPane({ dock: "shell", tabId: "chat:t2:sh:1", threadId: "t2" });
    expect(heard).toEqual(["t2"]);
  });

  it("ignores a request that names nothing", () => {
    const heard = vi.fn();
    onRevealPane(heard);
    revealPane({ dock: "web" });
    revealPane({ tabId: "chat:t1:web:1" });
    revealPane(null);
    expect(heard).not.toHaveBeenCalled();
  });

  it("keeps telling the others when one throws", () => {
    // A reveal that opens the panel and never picks the tab is worse than one
    // that does nothing at all.
    const after = vi.fn();
    onRevealPane(() => {
      throw new Error("unmounted");
    });
    onRevealPane(after);
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    expect(after).toHaveBeenCalledOnce();
  });

  it("stops telling a pane that unsubscribed", () => {
    const gone = vi.fn();
    onRevealPane(gone)();
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    expect(gone).not.toHaveBeenCalled();
  });
});

describe("a request made before anyone could hear it", () => {
  it("reaches the pane that opening the dock is what mounts", () => {
    // The whole failure this exists for: a closed dock mounts no panes, so the
    // request that opens it is made while nothing is subscribed. The agent
    // started a dev server and the person opening the terminal afterwards
    // found their tab strip untouched.
    revealPane({ dock: "shell", tabId: "chat:t1:job:8123", threadId: "t1" });
    const heard = [];
    onRevealPane((detail) => heard.push(detail.tabId));
    expect(heard).toEqual(["chat:t1:job:8123"]);
  });

  it("replays every tab that is waiting, newest last", () => {
    // Two background services started in one turn are two reveals a fraction
    // apart. Keeping only the newest showed one tab for two processes - the
    // agent could read three terminals while the person could see two.
    revealPane({ dock: "shell", tabId: "chat:t1:job:21728", threadId: "t1" });
    revealPane({ dock: "shell", tabId: "chat:t1:job:7732", threadId: "t1" });
    const heard = [];
    onRevealPane((detail) => heard.push(detail.tabId));
    expect(heard).toEqual(["chat:t1:job:21728", "chat:t1:job:7732"]);
  });

  it("does not replay the same tab twice for being asked for twice", () => {
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    const heard = [];
    onRevealPane((detail) => heard.push(detail.tabId));
    expect(heard).toEqual(["chat:t1:web:1"]);
  });

  it("keeps the browser's and the terminal's separate", () => {
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    revealPane({ dock: "shell", tabId: "chat:t1:sh:1", threadId: "t1" });
    const heard = [];
    onRevealPane((detail) => heard.push(detail.tabId));
    expect(heard.sort()).toEqual(["chat:t1:sh:1", "chat:t1:web:1"]);
  });

  it("keeps one conversation's waiting request out of another's", () => {
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    revealPane({ dock: "web", tabId: "chat:t2:web:1", threadId: "t2" });
    expect(pendingReveals("web", "t1").map((one) => one.tabId)).toEqual(["chat:t1:web:1"]);
    expect(pendingReveals("web", "t2").map((one) => one.tabId)).toEqual(["chat:t2:web:1"]);
  });

  it("is dropped once a pane has adopted it", () => {
    // Or a panel closed and reopened an hour later jumps back to whatever the
    // agent happened to be doing then.
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    clearReveal("web", "t1", "chat:t1:web:1");
    expect(pendingReveals("web", "t1")).toEqual([]);
    const heard = vi.fn();
    onRevealPane(heard);
    expect(heard).not.toHaveBeenCalled();
  });

  it("drops one tab without dropping the other waiting beside it", () => {
    revealPane({ dock: "shell", tabId: "chat:t1:job:1", threadId: "t1" });
    revealPane({ dock: "shell", tabId: "chat:t1:job:2", threadId: "t1" });
    clearReveal("shell", "t1", "chat:t1:job:1");
    expect(pendingReveals("shell", "t1").map((one) => one.tabId)).toEqual(["chat:t1:job:2"]);
  });

  it("drops the whole panel's worth when no tab is named", () => {
    revealPane({ dock: "shell", tabId: "chat:t1:job:1", threadId: "t1" });
    revealPane({ dock: "shell", tabId: "chat:t1:job:2", threadId: "t1" });
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    clearReveal("shell", "t1");
    expect(pendingReveals("shell", "t1")).toEqual([]);
    expect(pendingReveals("web", "t1")).toHaveLength(1);
  });

  it("survives a subscriber that throws on the replay", () => {
    revealPane({ dock: "web", tabId: "chat:t1:web:1", threadId: "t1" });
    expect(() =>
      onRevealPane(() => {
        throw new Error("mid-mount");
      })
    ).not.toThrow();
  });
});
