import { afterEach, describe, it, expect, vi } from "vitest";
import { applyEvent, runTurn, toHistory, textOf } from "./agent";

/**
 * The two pure halves of the transcript.
 *
 * `applyEvent` decides what a turn looks like while it happens; `toHistory`
 * decides what the model is told about it afterwards. Both fail quietly when
 * they are wrong - the first produces a transcript in the wrong order, the
 * second produces a model that has forgotten what a tool told it - so both are
 * worth pinning down.
 */

const blank = { id: "m1", parts: [] };

function fold(events, message = blank) {
  return events.reduce(applyEvent, message);
}

describe("applyEvent", () => {
  it("appends text to the run being written rather than making a part per token", () => {
    const message = fold([
      { type: "delta", text: "Hel" },
      { type: "delta", text: "lo" },
    ]);
    expect(message.parts).toHaveLength(1);
    expect(message.parts[0]).toEqual({ type: "text", text: "Hello" });
  });

  it("keeps reasoning apart from prose", () => {
    const message = fold([
      { type: "reasoning", text: "thinking" },
      { type: "delta", text: "answer" },
    ]);
    expect(message.parts.map((p) => p.type)).toEqual(["reasoning", "text"]);
    expect(textOf(message)).toBe("answer");
  });

  it("shows a tool the moment it starts, not when it finishes", () => {
    const message = fold([
      { type: "tool-start", callId: "c1", name: "read", title: "a.js", args: {} },
    ]);
    expect(message.parts[0]).toMatchObject({ type: "tool", callId: "c1", state: "running" });
  });

  it("fills in the result on the part that was already there", () => {
    const message = fold([
      { type: "tool-start", callId: "c1", name: "read", title: "a.js", args: {} },
      { type: "tool-end", callId: "c1", ok: true, output: "contents", durationMs: 12 },
    ]);
    expect(message.parts).toHaveLength(1);
    expect(message.parts[0]).toMatchObject({ state: "done", output: "contents", durationMs: 12 });
  });

  it("merges partial output into a running tool without replacing it", () => {
    const message = fold([
      { type: "tool-start", callId: "c1", name: "shell", title: "npm test", args: {} },
      { type: "tool-update", callId: "c1", metadata: { output: "running..." } },
      { type: "tool-update", callId: "c1", metadata: { output: "2 passed" } },
    ]);
    expect(message.parts[0].state).toBe("running");
    expect(message.parts[0].metadata.output).toBe("2 passed");
  });

  it("marks a failed call rather than dropping it", () => {
    const message = fold([
      { type: "tool-start", callId: "c1", name: "edit", title: "a.js", args: {} },
      { type: "tool-end", callId: "c1", ok: false, output: "File not found" },
    ]);
    expect(message.parts[0].state).toBe("failed");
    expect(message.parts[0].output).toBe("File not found");
  });

  it("ignores a result for a call it never saw start", () => {
    const message = fold([{ type: "tool-end", callId: "ghost", ok: true, output: "x" }]);
    expect(message.parts).toHaveLength(0);
  });

  it("puts text written after a tool call after the tool card", () => {
    const message = fold([
      { type: "delta", text: "Let me look. " },
      { type: "tool-start", callId: "c1", name: "read", title: "a.js", args: {} },
      { type: "tool-end", callId: "c1", ok: true, output: "x" },
      { type: "delta", text: "It has one line." },
    ]);
    expect(message.parts.map((p) => p.type)).toEqual(["text", "tool", "text"]);
    // The text parts stay separate, so the transcript reads in the order the
    // agent actually did things.
    expect(textOf(message)).toBe("Let me look. It has one line.");
  });

  it("drops what the person said mid-turn into the reply where they said it", () => {
    const message = fold([
      { type: "tool-start", callId: "c1", name: "read", title: "a.js", args: {} },
      { type: "steer", text: "not that file" },
      { type: "tool-end", callId: "c1", ok: true, output: "x" },
      { type: "delta", text: "Switching." },
    ]);
    expect(message.parts.map((p) => p.type)).toEqual(["tool", "steer", "text"]);
    // Their words are not the agent's. Counting them as prose would put them in
    // the preview line and in everything a copy button produces.
    expect(textOf(message)).toBe("Switching.");
  });
});

describe("toHistory", () => {
  it("passes a plain exchange through", () => {
    const history = toHistory([
      { role: "user", content: "hi" },
      { role: "agent", content: "hello", parts: [{ type: "text", text: "hello" }] },
    ]);
    expect(history).toEqual([
      { role: "user", content: "hi" },
      { role: "assistant", content: "hello" },
    ]);
  });

  it("drops a user turn with nothing in it", () => {
    expect(toHistory([{ role: "user", content: "  " }])).toEqual([]);
  });

  it("rebuilds a tool call and its result in the shape a provider wants", () => {
    const history = toHistory([
      { role: "user", content: "how many lines" },
      {
        role: "agent",
        parts: [
          { type: "text", text: "Let me look. " },
          {
            type: "tool",
            callId: "c1",
            name: "read",
            args: { filePath: "/a.js" },
            state: "done",
            output: "1: one",
          },
          { type: "text", text: "One line." },
        ],
      },
    ]);

    expect(history).toEqual([
      { role: "user", content: "how many lines" },
      {
        role: "assistant",
        content: "Let me look. ",
        toolCalls: [{ id: "c1", name: "read", arguments: '{"filePath":"/a.js"}' }],
      },
      { role: "tool", toolCallId: "c1", content: "1: one" },
      { role: "assistant", content: "One line." },
    ]);
  });

  it("carries a failed call's message back too", () => {
    const history = toHistory([
      {
        role: "agent",
        parts: [
          {
            type: "tool",
            callId: "c1",
            name: "read",
            args: {},
            state: "failed",
            output: "not found",
          },
        ],
      },
    ]);
    // A failure the model cannot see is a failure it repeats.
    expect(history[1]).toEqual({ role: "tool", toolCallId: "c1", content: "not found" });
  });

  it("leaves out a call that is still running", () => {
    const history = toHistory([
      {
        role: "agent",
        parts: [
          { type: "text", text: "working" },
          { type: "tool", callId: "c1", name: "shell", args: {}, state: "running" },
        ],
      },
    ]);
    // An unanswered tool call in the transcript is rejected by most providers,
    // so a turn interrupted mid-call must not send one.
    expect(history).toEqual([{ role: "assistant", content: "working" }]);
  });

  it("keeps several calls in one assistant message together with their results", () => {
    const history = toHistory([
      {
        role: "agent",
        parts: [
          { type: "tool", callId: "c1", name: "read", args: {}, state: "done", output: "a" },
          { type: "tool", callId: "c2", name: "read", args: {}, state: "done", output: "b" },
        ],
      },
    ]);
    expect(history[0].toolCalls).toHaveLength(2);
    expect(history[1].toolCallId).toBe("c1");
    expect(history[2].toolCallId).toBe("c2");
  });

  /**
   * A turn someone interrupted, sent again a turn later.
   *
   * The steered message is stored inside the reply it interrupted, which is
   * where it was said. Rebuilding the transcript has to put it back there: at
   * the end it reads as though the person waited until the agent was finished,
   * and at the start it reads as though they said it before any of the work
   * happened. Neither is what took place, and the model plans from it.
   */
  it("puts a steered message back where it interrupted, between two assistant turns", () => {
    const history = toHistory([
      { role: "user", content: "tidy up the config" },
      {
        role: "agent",
        parts: [
          { type: "text", text: "Reading it now. " },
          {
            type: "tool",
            callId: "c1",
            name: "read",
            args: { filePath: "/a.json" },
            state: "done",
            output: "{}",
          },
          { type: "steer", text: "not that one, the other config" },
          { type: "text", text: "Right, switching." },
        ],
      },
    ]);

    expect(history).toEqual([
      { role: "user", content: "tidy up the config" },
      {
        role: "assistant",
        content: "Reading it now. ",
        toolCalls: [{ id: "c1", name: "read", arguments: '{"filePath":"/a.json"}' }],
      },
      { role: "tool", toolCallId: "c1", content: "{}" },
      { role: "user", content: "not that one, the other config" },
      { role: "assistant", content: "Right, switching." },
    ]);
  });

  it("never lets a steered message split a tool call from its result", () => {
    // The pairing both transports are built on, and the one the Messages API
    // refuses outright when it is broken.
    const history = toHistory([
      {
        role: "agent",
        parts: [
          { type: "tool", callId: "c1", name: "read", args: {}, state: "done", output: "a" },
          { type: "steer", text: "stop reading" },
        ],
      },
    ]);
    expect(history.map((e) => e.role)).toEqual(["assistant", "tool", "user"]);
  });

  it("falls back to plain content for a message stored before parts existed", () => {
    const history = toHistory([{ role: "agent", content: "old reply" }]);
    expect(history).toEqual([{ role: "assistant", content: "old reply" }]);
  });
});

describe("attaching files to a message", () => {
  it("folds a text file into the message, named, rather than sending it blind", () => {
    const [entry] = toHistory([
      {
        role: "user",
        content: "what is wrong with this?",
        attachments: [{ id: "a1", kind: "text", name: "notes.csv", text: "a,b\n1,2" }],
      },
    ]);
    expect(entry.role).toBe("user");
    expect(entry.content).toContain("what is wrong with this?");
    expect(entry.content).toContain("notes.csv");
    expect(entry.content).toContain("a,b");
    // No parts: text needs none, and a text-only model must not be handed any.
    expect(entry.parts).toBeUndefined();
  });

  it("sends an image as a content part, with the question alongside it", () => {
    const [entry] = toHistory([
      {
        role: "user",
        content: "what is wrong with this?",
        attachments: [
          { id: "a1", kind: "image", name: "shot.png", dataUrl: "data:image/png;base64,iVBORw0K" },
        ],
      },
    ]);
    expect(entry.parts).toEqual([
      { type: "text", text: "what is wrong with this?" },
      { type: "image_url", image_url: { url: "data:image/png;base64,iVBORw0K" } },
    ]);
  });

  it("sends a picture with no words at all, because that is a real message", () => {
    const [entry] = toHistory([
      {
        role: "user",
        content: "",
        attachments: [
          { id: "a1", kind: "image", name: "shot.png", dataUrl: "data:image/png;base64,x" },
        ],
      },
    ]);
    expect(entry.parts[0]).toEqual({ type: "text", text: "Attached." });
    expect(entry.parts).toHaveLength(2);
  });
});

/**
 * A turn that cannot hang.
 *
 * Both of these are recoveries from a backend that has stopped answering,
 * which is not something the window can detect by watching - a turn that is
 * quiet because it is running a build looks exactly like one that is quiet
 * because nobody is ever going to speak again. Fake timers, because the real
 * waits are measured in minutes.
 */
describe("a turn that stops answering", () => {
  let listener = null;

  function bridge(overrides = {}) {
    listener = null;
    const calls = { cancelled: [] };
    globalThis.window = {
      agentAPI: {
        run: () => new Promise(() => {}),
        cancel: (id) => {
          calls.cancelled.push(id);
          return Promise.resolve({ cancelled: true });
        },
        active: () => Promise.resolve([]),
        onEvent: (fn) => {
          listener = fn;
          return () => {
            listener = null;
          };
        },
        ...overrides,
      },
    };
    return calls;
  }

  afterEach(() => {
    vi.useRealTimers();
    delete globalThis.window;
  });

  it("gives up on a start that never comes back, instead of typing forever", async () => {
    vi.useFakeTimers();
    bridge();
    const events = [];
    runTurn({ threadId: "t1", on: (event) => events.push(event) });

    await vi.advanceTimersByTimeAsync(44_000);
    expect(events).toEqual([]);

    await vi.advanceTimersByTimeAsync(2_000);
    expect(events).toHaveLength(1);
    expect(events[0].type).toBe("error");
    expect(events[0].message).toContain("could not be started");
  });

  it("stops a turn that arrives after it was given up on, rather than orphaning it", async () => {
    vi.useFakeTimers();
    let land = null;
    const calls = bridge({ run: () => new Promise((resolve) => (land = resolve)) });
    runTurn({ threadId: "t1", on: () => {} });

    await vi.advanceTimersByTimeAsync(46_000);
    land({ id: "turn-late" });
    await vi.advanceTimersByTimeAsync(0);

    expect(calls.cancelled).toEqual(["turn-late"]);
  });

  it("waits out a turn that is quiet but still running", async () => {
    vi.useFakeTimers();
    bridge({
      run: () => Promise.resolve({ id: "turn-1" }),
      active: () => Promise.resolve([{ id: "turn-1" }]),
    });
    const events = [];
    runTurn({ threadId: "t1", on: (event) => events.push(event) });
    await vi.advanceTimersByTimeAsync(0);

    // Twice round the silence timer: a long build is not a failure.
    await vi.advanceTimersByTimeAsync(260_000);
    expect(events.filter((e) => e.type === "error")).toEqual([]);
  });

  it("ends a turn that has gone quiet and is no longer running", async () => {
    vi.useFakeTimers();
    bridge({ run: () => Promise.resolve({ id: "turn-1" }), active: () => Promise.resolve([]) });
    const events = [];
    runTurn({ threadId: "t1", on: (event) => events.push(event) });
    await vi.advanceTimersByTimeAsync(0);

    await vi.advanceTimersByTimeAsync(130_000);
    const errors = events.filter((e) => e.type === "error");
    expect(errors).toHaveLength(1);
    expect(errors[0].message).toContain("stopped reporting");
  });

  it("does not fire either timer once the turn has finished on its own", async () => {
    vi.useFakeTimers();
    bridge({ run: () => Promise.resolve({ id: "turn-1" }), active: () => Promise.resolve([]) });
    const events = [];
    runTurn({ threadId: "t1", on: (event) => events.push(event) });
    await vi.advanceTimersByTimeAsync(0);

    listener({ id: "turn-1", type: "done", finish: "stop" });
    await vi.advanceTimersByTimeAsync(400_000);

    expect(events.filter((e) => e.type === "error")).toEqual([]);
  });

  it("keeps a live turn alive as long as it is saying something", async () => {
    vi.useFakeTimers();
    bridge({ run: () => Promise.resolve({ id: "turn-1" }), active: () => Promise.resolve([]) });
    const events = [];
    runTurn({ threadId: "t1", on: (event) => events.push(event) });
    await vi.advanceTimersByTimeAsync(0);

    // A token every minute for five minutes: well past the silence window,
    // and never silent.
    for (let i = 0; i < 5; i += 1) {
      await vi.advanceTimersByTimeAsync(60_000);
      listener({ id: "turn-1", type: "delta", text: "." });
    }
    expect(events.filter((e) => e.type === "error")).toEqual([]);
  });
});

/**
 * Naming who said what, in a conversation with several agents in it.
 *
 * Without this the agent about to speak reads an undifferentiated run of
 * assistant turns and cannot tell its own words from a colleague's, which is
 * the difference between a room and a monologue with several authors.
 */
describe("labelling a group transcript", () => {
  const label = (message) => (message.agentId === "a1" ? "Nova" : "Iris");

  it("carries who wrote each reply on the entry, not in the words", () => {
    const history = toHistory(
      [
        { role: "user", content: "who is on this?" },
        { role: "agent", agentId: "a1", content: "I am." },
        { role: "agent", agentId: "a2", content: "And so am I." },
      ],
      { label }
    );
    expect(history).toEqual([
      { role: "user", content: "who is on this?" },
      { role: "assistant", content: "I am.", agentId: "a1", name: "Nova" },
      { role: "assistant", content: "And so am I.", agentId: "a2", name: "Iris" },
    ]);
  });

  it("marks every chunk of a reply with tool calls in it", () => {
    const history = toHistory(
      [
        {
          role: "agent",
          agentId: "a1",
          parts: [
            { type: "text", text: "Looking." },
            { type: "tool", callId: "c1", name: "read", args: {}, output: "ok", state: "done" },
            { type: "text", text: "Found it." },
          ],
        },
      ],
      { label }
    );
    const said = history.filter((m) => m.role === "assistant");
    expect(said.map((m) => m.content)).toEqual(["Looking.", "Found it."]);
    expect(said.map((m) => m.agentId)).toEqual(["a1", "a1"]);
  });

  it("takes off a name the model wrote in front of its own reply", () => {
    const history = toHistory([{ role: "agent", agentId: "a2", content: "Iris: mine" }], { label });
    expect(history[0].content).toBe("mine");
  });

  it("leaves a turn that passed out of the history", () => {
    const history = toHistory(
      [
        { role: "agent", agentId: "a1", content: "mine" },
        { role: "agent", agentId: "a2", content: "pass", quiet: true },
        { role: "user", content: "carry on" },
      ],
      { label }
    );
    expect(history).toEqual([
      { role: "assistant", content: "mine", agentId: "a1", name: "Nova" },
      { role: "user", content: "carry on" },
    ]);
  });

  it("leaves the transcript alone when there is no label", () => {
    expect(toHistory([{ role: "agent", content: "I am." }])[0]).toEqual({
      role: "assistant",
      content: "I am.",
    });
  });
});

describe("a compaction in the transcript", () => {
  it("replaces everything before it with its note", () => {
    const history = toHistory([
      { role: "user", content: "first" },
      { role: "agent", content: "second" },
      { role: "system", content: "Summarised 2 messages.", summary: "They talked about a table." },
      { role: "user", content: "carry on" },
    ]);
    expect(history).toHaveLength(2);
    expect(history[0].role).toBe("user");
    expect(history[0].content).toContain("They talked about a table.");
    expect(history[0].content).not.toContain("first");
    expect(history[1]).toEqual({ role: "user", content: "carry on" });
  });

  it("leaves a local note out of the history entirely", () => {
    expect(
      toHistory([
        { role: "user", content: "hello" },
        { role: "system", content: "Automatic compaction is on." },
      ])
    ).toEqual([{ role: "user", content: "hello" }]);
  });

  it("keeps only the last compaction when there are several", () => {
    const history = toHistory([
      { role: "user", content: "a" },
      { role: "system", content: "note", summary: "first summary" },
      { role: "user", content: "b" },
      { role: "system", content: "note", summary: "second summary" },
      { role: "user", content: "c" },
    ]);
    expect(history).toHaveLength(2);
    expect(history[0].content).toContain("second summary");
    expect(history[0].content).not.toContain("first summary");
  });
});
