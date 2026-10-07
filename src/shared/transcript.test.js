import { describe, it, expect } from "vitest";
import { applyEvent, isThreadWorking, todosOf } from "@shared/transcript";

const list = (...items) => items.map((content, i) => ({ id: `t${i}`, content, status: "pending" }));

describe("the current task list", () => {
  it("is empty when nothing wrote one", () => {
    expect(todosOf([{ role: "user", content: "hi" }])).toEqual([]);
    expect(todosOf()).toEqual([]);
  });

  it("reads the newest todowrite in the thread", () => {
    const messages = [
      {
        role: "agent",
        parts: [
          { type: "tool", callId: "1", name: "todowrite", metadata: { todos: list("one", "two") } },
        ],
      },
      {
        role: "agent",
        parts: [
          { type: "tool", callId: "2", name: "todowrite", metadata: { todos: list("three") } },
        ],
      },
    ];
    expect(todosOf(messages).map((t) => t.content)).toEqual(["three"]);
  });

  it("prefers the later call inside one reply", () => {
    const messages = [
      {
        role: "agent",
        parts: [
          { type: "tool", callId: "1", name: "todowrite", metadata: { todos: list("first") } },
          { type: "tool", callId: "2", name: "shell" },
          { type: "tool", callId: "3", name: "todowrite", metadata: { todos: list("second") } },
        ],
      },
    ];
    expect(todosOf(messages).map((t) => t.content)).toEqual(["second"]);
  });

  it("shows the list while the call is still running", () => {
    // tool-start carries the arguments; metadata only arrives at tool-end, and
    // the header should not wait a second for a list it already has.
    const message = applyEvent(
      { parts: [] },
      { type: "tool-start", callId: "1", name: "todowrite", args: { todos: list("open the page") } }
    );
    expect(todosOf([message]).map((t) => t.content)).toEqual(["open the page"]);
  });

  it("reads a top-level tool block too", () => {
    const messages = [{ type: "tool", name: "todowrite", metadata: { todos: list("seeded") } }];
    expect(todosOf(messages).map((t) => t.content)).toEqual(["seeded"]);
  });

  it("ignores a todowrite that carried no list", () => {
    const messages = [
      { role: "agent", parts: [{ type: "tool", callId: "1", name: "todowrite", args: {} }] },
    ];
    expect(todosOf(messages)).toEqual([]);
  });
});

/**
 * The two halves of "is anything still happening". Both used to be answered by
 * one field on the message, and both were wrong in the same conversation: a
 * call left spinning after its turn had died, and a turn declared finished
 * while one of its calls was suspended on a permission card.
 */
describe("a turn that ends", () => {
  const running = () => ({
    parts: [
      { type: "tool", callId: "1", name: "shell", state: "done", output: "ok" },
      { type: "tool", callId: "2", name: "ls", state: "running" },
    ],
  });

  it("leaves no call still running", () => {
    const done = applyEvent(running(), { type: "done" });
    expect(done.parts.map((p) => p.state)).toEqual(["done", "failed"]);
    expect(done.parts[1].output).toBe("The turn ended before this call finished.");
  });

  it("says so differently when the person stopped it", () => {
    const stopped = applyEvent(running(), { type: "done", stopped: "cancelled" });
    expect(stopped.parts[1].output).toBe("The turn was stopped before this call finished.");
    expect(stopped.state).toBe("cancelled");
  });

  it("settles them on an error too, and keeps the error last", () => {
    const failed = applyEvent(running(), { type: "error", message: "the key died" });
    expect(failed.parts[1].state).toBe("failed");
    expect(failed.parts[failed.parts.length - 1]).toEqual({ type: "error", text: "the key died" });
  });

  it("keeps whatever output a call had already streamed", () => {
    const message = {
      parts: [
        { type: "tool", callId: "1", name: "shell", state: "running", output: "half a build" },
      ],
    };
    expect(applyEvent(message, { type: "done" }).parts[0].output).toBe("half a build");
  });

  it("hands back the same parts when nothing was running", () => {
    const message = { parts: [{ type: "tool", callId: "1", name: "ls", state: "done" }] };
    expect(applyEvent(message, { type: "done" }).parts).toBe(message.parts);
  });
});

describe("whether a conversation is working", () => {
  it("is not, for a settled transcript", () => {
    expect(isThreadWorking([{ role: "agent", status: "sent" }])).toBe(false);
    expect(isThreadWorking()).toBe(false);
  });

  it("is, while a reply is still arriving", () => {
    expect(isThreadWorking([{ role: "agent", status: "streaming" }])).toBe(true);
  });

  it("is, while a call inside a settled reply is still running", () => {
    // The case that put a timestamp under a reply whose tool card said Running.
    const blocks = [
      { role: "agent", status: "sent", parts: [{ type: "tool", callId: "1", state: "running" }] },
    ];
    expect(isThreadWorking(blocks)).toBe(true);
  });

  it("is, for a top-level tool block from a seeded thread", () => {
    expect(isThreadWorking([{ type: "tool", state: "running" }])).toBe(true);
  });

  it("is, while a question is waiting on the person", () => {
    // A permission card is a tool suspended mid-call. Nothing in the
    // transcript says so, which is why the prompts are asked separately.
    expect(
      isThreadWorking([{ role: "agent", status: "sent" }], [{ question: { id: "ask-1" } }])
    ).toBe(true);
  });
});

describe("a call that starts twice", () => {
  const start = { type: "tool-start", callId: "c1", name: "shell", args: { command: "ls" } };

  it("has one card, not two", () => {
    const once = applyEvent({ role: "agent", parts: [] }, start);
    const twice = applyEvent(once, start);
    expect(twice.parts.filter((p) => p.type === "tool")).toHaveLength(1);
  });

  it("keeps the state the card already reached", () => {
    let message = applyEvent({ role: "agent", parts: [] }, start);
    message = applyEvent(message, { type: "tool-end", callId: "c1", ok: true, output: "a\nb" });
    message = applyEvent(message, start);
    expect(message.parts[0]).toMatchObject({ callId: "c1", state: "done", output: "a\nb" });
  });
});
