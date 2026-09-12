import { describe, it, expect } from "vitest";
import {
  asActivityRecords,
  asMessageRecords,
  byTimeDesc,
  byUpdatedDesc,
  settleAgent,
  settleAgents,
  settleOnLoad,
  reconcileMessages,
  mergeRows,
  syncCollection,
  syncDocument,
  fingerprint,
} from "./workspace-sync";
import { SOURCES } from "./workspace-store";

/**
 * These are the rules that lose data when they are wrong, so they are tested
 * against a fake client that records exactly which calls were made. What
 * matters is not that a write happened but that the RIGHT writes happened and
 * the wrong ones did not - a sync that rewrites every file on every keystroke
 * passes a naive test and ruins a folder.
 */

function fakeClient(initial = {}) {
  const calls = { put: [], remove: [], writeDocument: [] };
  const documents = { ...initial };
  return {
    calls,
    documents,
    async put(name, record) {
      calls.put.push(`${name}/${record.id}`);
      return record;
    },
    async remove(name, id) {
      calls.remove.push(`${name}/${id}`);
    },
    async readDocument(key, fallback) {
      return documents[key] ?? fallback;
    },
    async writeDocument(key, value) {
      calls.writeDocument.push(key);
      documents[key] = value;
      return value;
    },
  };
}

describe("syncCollection", () => {
  it("writes everything the first time", async () => {
    const client = fakeClient();
    const marks = await syncCollection(
      client,
      "threads",
      [{ id: "a", title: "A" }, { id: "b", title: "B" }],
      new Map()
    );
    expect(client.calls.put).toEqual(["threads/a", "threads/b"]);
    expect(marks.size).toBe(2);
  });

  it("writes nothing when nothing moved", async () => {
    const client = fakeClient();
    const rows = [{ id: "a", title: "A" }, { id: "b", title: "B" }];
    const marks = await syncCollection(client, "threads", rows, new Map());
    client.calls.put.length = 0;

    await syncCollection(client, "threads", rows.map((r) => ({ ...r })), marks);
    expect(client.calls.put).toEqual([]);
    expect(client.calls.remove).toEqual([]);
  });

  it("writes only the record that changed", async () => {
    const client = fakeClient();
    const marks = await syncCollection(
      client,
      "threads",
      [{ id: "a", title: "A" }, { id: "b", title: "B" }],
      new Map()
    );
    client.calls.put.length = 0;

    await syncCollection(
      client,
      "threads",
      [{ id: "a", title: "A" }, { id: "b", title: "B changed" }],
      marks
    );
    expect(client.calls.put).toEqual(["threads/b"]);
  });

  it("deletes a record that is gone from the app", async () => {
    const client = fakeClient();
    const marks = await syncCollection(
      client,
      "threads",
      [{ id: "a" }, { id: "b" }],
      new Map()
    );
    const next = await syncCollection(client, "threads", [{ id: "a" }], marks);
    expect(client.calls.remove).toEqual(["threads/b"]);
    expect([...next.keys()]).toEqual(["a"]);
  });

  it("empties the folder when the app has nothing left", async () => {
    const client = fakeClient();
    const marks = await syncCollection(client, "memory", [{ id: "m1" }], new Map());
    await syncCollection(client, "memory", [], marks);
    expect(client.calls.remove).toEqual(["memory/m1"]);
  });
});

describe("syncDocument", () => {
  it("merges into the document rather than replacing it", async () => {
    const client = fakeClient({ "settings.identity": { theme: "dark" } });
    await syncDocument(client, "settings.identity", "user", { name: "Elias" }, undefined);
    expect(client.documents["settings.identity"]).toEqual({
      theme: "dark",
      user: { name: "Elias" },
    });
  });

  it("does not write when the value is unchanged", async () => {
    const client = fakeClient();
    const value = { name: "Elias" };
    const mark = await syncDocument(client, "settings.identity", "user", value, undefined);
    client.calls.writeDocument.length = 0;

    await syncDocument(client, "settings.identity", "user", { ...value }, mark);
    expect(client.calls.writeDocument).toEqual([]);
  });
});

describe("shape conversions", () => {
  it("turns the message map into one record per thread", () => {
    const records = asMessageRecords({ "thr-1": [{ id: "m1" }], "thr-2": [] });
    expect(records).toEqual([
      { id: "thr-1", threadId: "thr-1", messages: [{ id: "m1" }] },
      { id: "thr-2", threadId: "thr-2", messages: [] },
    ]);
  });

  it("groups activity into day files", () => {
    const records = asActivityRecords([
      { id: "a", at: "2026-09-01T10:00:00Z" },
      { id: "b", at: "2026-09-01T23:00:00Z" },
      { id: "c", at: "2026-09-02T01:00:00Z" },
    ]);
    expect(records.map((r) => r.id)).toEqual(["2026-09-01", "2026-09-02"]);
    expect(records[0].events).toHaveLength(2);
  });

  it("orders lists newest first", () => {
    expect(byUpdatedDesc([{ updatedAt: "2026-01-01" }, { updatedAt: "2026-02-01" }])[0].updatedAt)
      .toBe("2026-02-01");
    expect(byTimeDesc([{ at: "2026-01-01" }, { at: "2026-02-01" }])[0].at).toBe("2026-02-01");
  });
});

describe("settleOnLoad", () => {
  it("keeps partial text and marks it stopped", () => {
    const [message] = settleOnLoad([
      { id: "m1", role: "agent", status: "streaming", content: "Half an ans" },
    ]);
    expect(message.status).toBe("sent");
    expect(message.stopped).toBe(true);
    expect(message.content).toBe("Half an ans");
  });

  it("turns a reply that produced nothing into a visible failure", () => {
    const [message] = settleOnLoad([
      { id: "m1", role: "agent", status: "streaming", content: "" },
    ]);
    expect(message.status).toBe("error");
    expect(message.error).toMatch(/interrupted/i);
  });

  it("leaves settled messages alone, and returns the same array", () => {
    const rows = [
      { id: "m1", role: "user", status: "sent", content: "hi" },
      { id: "m2", role: "agent", status: "error", content: "" },
    ];
    expect(settleOnLoad(rows)).toBe(rows);
  });

  it("survives a missing list", () => {
    expect(settleOnLoad(undefined)).toEqual([]);
  });
});

/**
 * The bug this exists for: a reply died mid-sentence, saying it had been
 * interrupted, while the request behind it was streaming perfectly well. The
 * store mirrors messages to disk as they arrive, main announces every record
 * it writes, and the window that wrote it hears its own announcement and
 * re-reads - so a live turn read its own half-written self back and settled
 * itself as abandoned.
 */
describe("reconcileMessages", () => {
  const live = { id: "m2", role: "agent", status: "streaming", content: "Half an ans" };

  it("does not settle a turn this window is still streaming", () => {
    const held = [{ id: "m1", role: "user", status: "sent", content: "hi" }, live];
    // What the disk has: the same turn, as it looked one mirror pass ago.
    const fromDisk = [
      { id: "m1", role: "user", status: "sent", content: "hi" },
      { id: "m2", role: "agent", status: "streaming", content: "Half" },
    ];
    const [, reply] = reconcileMessages(held, fromDisk);
    expect(reply.status).toBe("streaming");
    expect(reply.error).toBeUndefined();
    // And the fresher text wins, so the visible reply does not rewind either.
    expect(reply.content).toBe("Half an ans");
  });

  it("keeps a reply that has not reached the disk yet", () => {
    // The first message of a new thread, in the gap between being spoken and
    // being written down.
    const merged = reconcileMessages([live], []);
    expect(merged).toEqual([live]);
  });

  it("still settles a turn nothing here knows about", () => {
    // The case the settling rule was written for: an abandoned turn from a
    // previous run, or another window. Nobody is streaming it now.
    const [message] = reconcileMessages([{ id: "m1", role: "user", status: "sent", content: "hi" }], [
      { id: "old", role: "agent", status: "streaming", content: "" },
    ]);
    expect(message.status).toBe("error");
    expect(message.error).toMatch(/interrupted/i);
  });

  it("settles everything when there is nothing held, which is what a load is", () => {
    const [message] = reconcileMessages([], [
      { id: "m1", role: "agent", status: "streaming", content: "" },
    ]);
    expect(message.status).toBe("error");
  });

  it("keeps the question that has not reached the disk yet", () => {
    // The bug this is here for: the whole map was replaced on a live re-read,
    // and only `streaming` messages were carried over - so a message that had
    // just been sent disappeared off the screen while the reply to it carried
    // on arriving underneath, leaving a named thread with an empty transcript.
    const asked = { id: "m9", role: "user", status: "sent", content: "make me an account" };
    const merged = reconcileMessages([asked], []);
    expect(merged).toEqual([asked]);
  });

  it("takes the disk copy for anything that is not mid-flight", () => {
    const held = [{ id: "m1", role: "agent", status: "sent", content: "stale" }];
    const [message] = reconcileMessages(held, [
      { id: "m1", role: "agent", status: "sent", content: "edited by hand" },
    ]);
    expect(message.content).toBe("edited by hand");
  });
});

describe("settleAgents", () => {
  it("drops the dead capabilities field", () => {
    const [agent] = settleAgents([{ id: "a1", name: "Atlas", capabilities: ["browser"] }]);
    expect(agent).not.toHaveProperty("capabilities");
    expect(agent.name).toBe("Atlas");
  });

  it("clears a model no configured provider offers", () => {
    const [agent] = settleAgents([{ id: "a1", model: "claude-opus-4-6" }], ["groq/gpt"]);
    expect(agent.model).toBe("");
  });

  it("keeps a model the workspace can reach", () => {
    const [agent] = settleAgents([{ id: "a1", model: "groq/gpt" }], ["groq/gpt"]);
    expect(agent.model).toBe("groq/gpt");
  });

  it("leaves the model alone when nothing is configured yet", () => {
    const [agent] = settleAgents([{ id: "a1", model: "groq/gpt" }], []);
    expect(agent.model).toBe("groq/gpt");
  });

  it("returns the same array when there is nothing to settle", () => {
    const rows = [{ id: "a1", name: "Atlas" }];
    expect(settleAgents(rows)).toBe(rows);
  });
});

describe("an agent that has never been used", () => {
  it("does not report a last-active time it cannot have earned", () => {
    // An agent written by another agent carries whatever timestamp its author
    // felt like. One arrived stamped three hours before it existed, and the
    // detail page introduced a brand new agent as "active 3h ago".
    const invented = {
      id: "agent-codebot",
      name: "CodeBot",
      lastActiveAt: "2026-09-03T05:10:00Z",
      stats: { messages: 0 },
    };
    expect(settleAgent(invented).lastActiveAt).toBe(null);
  });

  it("counts a routine run as work, not only a conversation", () => {
    // An agent whose whole job is a routine every morning answers nobody. The
    // rule used to look at the message count alone, so that agent had its
    // last-active time wiped on every launch and its page said it had never
    // run anything.
    const scheduled = {
      id: "agent-nightly",
      name: "Nightly",
      lastActiveAt: "2026-09-03T05:10:00Z",
      stats: { messages: 0, routinesRun: 6, tokensUsed: 12_000 },
    };
    expect(settleAgent(scheduled)).toBe(scheduled);
  });

  it("keeps the counters exactly as they were written", () => {
    // The numbers on an agent's page are only worth having if they are the
    // same numbers after a restart. They travel in the record itself, so what
    // this guards is that nothing on the way in quietly rewrites them.
    const rows = [{ id: "a1", name: "Atlas", stats: { messages: 3, routinesRun: 2, tokensUsed: 900 } }];
    const [agent] = settleAgents(rows);
    expect(agent.stats).toEqual({ messages: 3, routinesRun: 2, tokensUsed: 900 });
  });

  it("leaves the time alone once the agent has actually said something", () => {
    const used = {
      id: "agent-codebot",
      name: "CodeBot",
      lastActiveAt: "2026-09-03T05:10:00Z",
      stats: { messages: 4 },
    };
    expect(settleAgent(used)).toBe(used);
  });
});

describe("mergeRows", () => {
  it("is the disk's list when this window holds nothing", () => {
    const rows = [{ id: "a" }];
    expect(mergeRows([], rows)).toBe(rows);
    expect(mergeRows(undefined, rows)).toBe(rows);
  });

  it("keeps a row the folder has never heard of", () => {
    // A draft thread has no file by design, and a conversation started a
    // moment ago has not been written yet. Neither may be deleted by a re-read
    // that something else triggered.
    const held = [{ id: "written", title: "old" }, { id: "draft", title: "New chat" }];
    const merged = mergeRows(held, [{ id: "written", title: "renamed" }]);
    expect(merged.map((row) => row.id).sort()).toEqual(["draft", "written"]);
    expect(merged.find((row) => row.id === "written").title).toBe("renamed");
  });

  it("lets the folder correct a row this window has", () => {
    const merged = mergeRows([{ id: "a", title: "mine" }], [{ id: "a", title: "theirs" }]);
    expect(merged).toEqual([{ id: "a", title: "theirs" }]);
  });
});

/**
 * The rules as the store actually applies them.
 *
 * A re-read is triggered by anything that writes to the folder - this window's
 * own mirror, an agent, a routine firing every five minutes - and it used to
 * replace the whole of both collections with whatever was on disk at that
 * instant. A conversation being typed into had not been written yet, so it was
 * deleted off the screen: the thread stayed in the sidebar with the name it had
 * just been given and the transcript went back to the starter prompts.
 */
describe("a live re-read", () => {
  const state = (threads, messages) => {
    const held = { threads, messages };
    return {
      held,
      setThreads: (fn) => {
        held.threads = typeof fn === "function" ? fn(held.threads) : fn;
      },
      setMessages: (fn) => {
        held.messages = typeof fn === "function" ? fn(held.messages) : fn;
      },
    };
  };

  const sourceFor = (name) => SOURCES.find((one) => one.collection === name);

  it("does not delete the conversation that has not been written yet", () => {
    const fresh = [
      { id: "m1", role: "user", status: "sent", content: "make me an account" },
      { id: "m2", role: "agent", status: "streaming", content: "" },
    ];
    const set = state(
      [
        { id: "t-old", title: "Yesterday", updatedAt: "2026-09-02T10:00:00.000Z" },
        { id: "t-new", title: "Account signup", updatedAt: "2026-09-03T10:00:00.000Z" },
      ],
      { "t-old": [{ id: "old", role: "user", status: "sent", content: "hi" }], "t-new": fresh }
    );

    // What the folder holds: everything except the conversation started a
    // moment ago, plus the routine's thread that triggered the announcement.
    const rows = [
      { id: "t-old", title: "Yesterday", updatedAt: "2026-09-02T10:00:00.000Z" },
      { id: "routine-r1", title: "Routine: Folder check", updatedAt: "2026-09-03T09:00:00.000Z" },
    ];
    sourceFor("threads").apply(rows, set);
    sourceFor("messages").apply(
      [
        { id: "t-old", messages: [{ id: "old", role: "user", status: "sent", content: "hi" }] },
        { id: "routine-r1", messages: [] },
      ],
      set
    );

    expect(set.held.threads.map((t) => t.id).sort()).toEqual(["routine-r1", "t-new", "t-old"]);
    expect(set.held.messages["t-new"]).toEqual(fresh);
    expect(set.held.messages["routine-r1"]).toEqual([]);
  });

  it("still lets the folder correct what this window holds", () => {
    const set = state([{ id: "t1", title: "old name", updatedAt: "2026-09-03T10:00:00.000Z" }], {
      t1: [{ id: "m1", role: "agent", status: "sent", content: "stale" }],
    });
    sourceFor("threads").apply(
      [{ id: "t1", title: "new name", updatedAt: "2026-09-03T11:00:00.000Z" }],
      set
    );
    sourceFor("messages").apply(
      [{ id: "t1", messages: [{ id: "m1", role: "agent", status: "sent", content: "edited" }] }],
      set
    );
    expect(set.held.threads[0].title).toBe("new name");
    expect(set.held.messages.t1[0].content).toBe("edited");
  });
});

describe("a call the interrupted turn never finished", () => {
  it("is settled with the message that held it", () => {
    // The status was only half the jam: everything that asks whether a
    // conversation is busy reads the part's own state as well.
    const [reply] = settleOnLoad([
      {
        id: "m1",
        role: "agent",
        status: "streaming",
        content: "Reading",
        parts: [
          { type: "tool", callId: "1", state: "done" },
          { type: "tool", callId: "2", state: "running" },
        ],
      },
    ]);
    expect(reply.parts.map((part) => part.state)).toEqual(["done", "failed"]);
    expect(reply.parts[1].output).toBe("This call was interrupted and never finished.");
  });

  it("leaves the parts alone when none of them was running", () => {
    const parts = [{ type: "tool", callId: "1", state: "done" }];
    const [reply] = settleOnLoad([
      { id: "m1", role: "agent", status: "streaming", content: "done", parts },
    ]);
    expect(reply.parts).toBe(parts);
  });
});

/**
 * A conversation nobody asked to keep.
 *
 * Temporary was a dashed border in the composer and nothing else: the promise
 * was that this one is not written down, and every message in it went to the
 * folder like any other. These are the two rules that make it true, and the
 * one that used to quietly undo it.
 */
describe("a temporary conversation", () => {
  const state = (threads, messages) => {
    const held = { threads, messages };
    return {
      held,
      setThreads: (fn) => {
        held.threads = typeof fn === "function" ? fn(held.threads) : fn;
      },
      setMessages: (fn) => {
        held.messages = typeof fn === "function" ? fn(held.messages) : fn;
      },
    };
  };
  const sourceFor = (name) => SOURCES.find((one) => one.collection === name);

  const held = {
    threads: [
      { id: "t-kept", title: "Kept", updatedAt: "2026-09-03T10:00:00.000Z" },
      { id: "t-temp", title: "Just asking", temporary: true, updatedAt: "2026-09-03T11:00:00.000Z" },
      { id: "t-draft", title: "New chat", draft: true, updatedAt: "2026-09-03T12:00:00.000Z" },
    ],
    messages: {
      "t-kept": [{ id: "a", role: "user", content: "hi" }],
      "t-temp": [{ id: "b", role: "user", content: "do not write this down" }],
      "t-draft": [],
    },
  };

  it("is never written to the folder, and neither are its messages", () => {
    expect(sourceFor("threads").read(held).map((t) => t.id)).toEqual(["t-kept"]);
    expect(sourceFor("messages").read(held).map((r) => r.id)).toEqual(["t-kept"]);
  });

  it("survives an announcement of somebody else's write", () => {
    // The settle forces `temporary: false`, because a file claiming to be one
    // is a contradiction. Run over the merged list it hit the rows that have
    // no file - which turned every temporary conversation, and every draft,
    // into one the next mirror wrote down.
    const set = state(held.threads, held.messages);
    sourceFor("threads").apply(
      [{ id: "t-kept", title: "Kept", updatedAt: "2026-09-03T10:00:00.000Z" }],
      set
    );
    const byId = Object.fromEntries(set.held.threads.map((t) => [t.id, t]));
    expect(byId["t-temp"].temporary).toBe(true);
    expect(byId["t-draft"].draft).toBe(true);
  });

  it("does not believe a file that claims to be one", () => {
    const set = state([], {});
    sourceFor("threads").apply([{ id: "t-odd", title: "Hand edited", temporary: true, draft: true }], set);
    expect(set.held.threads[0].temporary).toBe(false);
    expect(set.held.threads[0].draft).toBe(false);
  });
});

/**
 * The one delete the mirror must refuse.
 *
 * What was on disk: fifteen message files holding real conversations, and an
 * empty threads folder. A thread record is deleted when it leaves the state,
 * and the only legitimate way it leaves is with its messages - so a record
 * whose messages are still held is kept whatever the state says.
 */
describe("a thread whose messages are still held", () => {
  const client = () => {
    const ops = [];
    return {
      ops,
      put: async (name, record) => ops.push(["put", name, record.id]),
      remove: async (name, id) => ops.push(["remove", name, id]),
    };
  };
  const threads = SOURCES.find((one) => one.collection === "threads");

  it("is not deleted from the folder, and stays marked as written", async () => {
    const c = client();
    const marks = new Map([["t1", "old"]]);
    const state = { threads: [], messages: { t1: [{ id: "m", role: "user", content: "hi" }] } };
    const next = await syncCollection(c, "threads", threads.read(state), marks, {
      keep: (id) => threads.keep(state, id),
    });
    expect(c.ops).toEqual([]);
    expect(next.get("t1")).toBe("old");
  });

  it("is deleted once the messages went with it, which is what deleting a conversation does", async () => {
    const c = client();
    const marks = new Map([["t1", "old"]]);
    const state = { threads: [], messages: {} };
    const next = await syncCollection(c, "threads", threads.read(state), marks, {
      keep: (id) => threads.keep(state, id),
    });
    expect(c.ops).toEqual([["remove", "threads", "t1"]]);
    expect(next.has("t1")).toBe(false);
  });

  it("does not shield an abandoned draft, which has an empty list and no file", () => {
    expect(threads.keep({ messages: { t1: [] } }, "t1")).toBe(false);
  });
});

describe("the cost of mirroring a long conversation", () => {
  it("gives back the same record while the messages have not changed", () => {
    const messages = { t1: [{ id: "m1" }], t2: [{ id: "m2" }] };
    const first = asMessageRecords(messages);
    const second = asMessageRecords(messages);
    expect(second[0]).toBe(first[0]);
    expect(second[1]).toBe(first[1]);
  });

  it("gives back a new record when a conversation grows", () => {
    const messages = { t1: [{ id: "m1" }] };
    const first = asMessageRecords(messages);
    const grown = asMessageRecords({ t1: [...messages.t1, { id: "m2" }] });
    expect(grown[0]).not.toBe(first[0]);
  });

  it("does not hash a record it was handed last time", async () => {
    const client = fakeClient();
    const seen = new Map();
    const messages = { t1: [{ id: "m1", content: "x" }] };

    const marks = await syncCollection(client, "messages", asMessageRecords(messages), new Map(), { seen });
    expect(client.calls.put).toHaveLength(1);

    // The same state, a tick later. Nothing is written and, more to the point,
    // nothing is serialised: the record is the same object.
    const again = await syncCollection(client, "messages", asMessageRecords(messages), marks, { seen });
    expect(client.calls.put).toHaveLength(1);
    expect(again).toBe(marks);
  });

  it("still notices a change when the shortcut is in place", async () => {
    const client = fakeClient();
    const seen = new Map();
    const marks = await syncCollection(client, "messages", asMessageRecords({ t1: [{ id: "m1" }] }), new Map(), { seen });
    await syncCollection(
      client,
      "messages",
      asMessageRecords({ t1: [{ id: "m1" }, { id: "m2" }] }),
      marks,
      { seen }
    );
    expect(client.calls.put).toHaveLength(2);
  });
});

describe("fingerprint", () => {
  it("keeps nothing of the record it measured", () => {
    const big = { id: "t", messages: [{ content: "x".repeat(50000) }] };
    expect(fingerprint(big).length).toBeLessThan(40);
  });

  it("changes when the record does", () => {
    expect(fingerprint({ a: 1 })).not.toBe(fingerprint({ a: 2 }));
    expect(fingerprint({ a: 1 })).toBe(fingerprint({ a: 1 }));
  });

  it("tells apart records that differ only late in a long string", () => {
    const one = { text: `${"a".repeat(20000)}x` };
    const two = { text: `${"a".repeat(20000)}y` };
    expect(fingerprint(one)).not.toBe(fingerprint(two));
  });
});
