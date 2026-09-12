import { describe, expect, it } from "vitest";
import { PREFERENCE_DEFAULTS } from "./appearance.js";
import {
  settleComputer,
  settleComputers,
  settleMemory,
  settleRoutine,
  settleThread,
  settleThreads,
  settleUser,
} from "./settle.js";

describe("settleComputer", () => {
  it("gives a hand-written record the fields the screens read without a guard", () => {
    const computer = settleComputer({ id: "cmp-1", name: "Box" });
    expect(computer.usage).toEqual({ cpuPct: 0, memPct: 0, diskPct: 0 });
    expect(computer.specs).toEqual({ cpu: 0, memoryGb: 0, diskGb: 0 });
    expect(computer.assignedAgentIds).toEqual([]);
    expect(computer.snapshotCount).toBe(0);
  });

  it("survives every field being the wrong type", () => {
    const computer = settleComputer({
      id: "cmp-1",
      name: { first: "Box" },
      usage: null,
      specs: "8 cores",
      assignedAgentIds: "agent-atlas",
      snapshotCount: "twelve",
      status: "melting",
    });
    expect(computer.name).toBe("");
    expect(computer.usage.cpuPct).toBe(0);
    expect(computer.specs.cpu).toBe(0);
    expect(computer.assignedAgentIds).toEqual([]);
    expect(computer.snapshotCount).toBe(0);
    expect(computer.status).toBe("stopped");
  });

  it("clamps a meter into the range the bar can draw", () => {
    const computer = settleComputer({ id: "c", usage: { cpuPct: 4000, memPct: -3, diskPct: 55 } });
    expect(computer.usage).toEqual({ cpuPct: 100, memPct: 0, diskPct: 55 });
  });

  it("keeps a record that was already right, object identity included", () => {
    const computer = {
      id: "cmp-1",
      name: "Box",
      status: "running",
      provider: "docker",
      usage: { cpuPct: 10, memPct: 20, diskPct: 30 },
      specs: { cpu: 4, memoryGb: 16, diskGb: 80 },
      assignedAgentIds: ["agent-atlas"],
      snapshotCount: 2,
    };
    expect(settleComputer(computer)).toBe(computer);
  });

  it("keeps the same array when no record in it moved", () => {
    const rows = [
      {
        id: "cmp-1",
        name: "Box",
        status: "running",
        provider: "docker",
        usage: { cpuPct: 1, memPct: 2, diskPct: 3 },
        specs: { cpu: 1, memoryGb: 2, diskGb: 3 },
        assignedAgentIds: [],
        snapshotCount: 0,
      },
    ];
    expect(settleComputers(rows)).toBe(rows);
  });

  it("repairs a record rather than dropping it", () => {
    const rows = settleComputers([{ id: "cmp-broken" }, null, { id: "cmp-fine", usage: {} }]);
    expect(rows).toHaveLength(3);
    expect(rows[0].id).toBe("cmp-broken");
  });
});

describe("settleRoutine", () => {
  it("reads a run with no timestamp as never run", () => {
    expect(settleRoutine({ id: "r", lastRun: {} }).lastRun).toBeNull();
    expect(settleRoutine({ id: "r", lastRun: { status: "ok" } }).lastRun).toEqual({
      status: "ok",
      at: null,
    });
  });

  it("defaults an unstated routine to enabled and drops non-string tags", () => {
    const routine = settleRoutine({ id: "r", tags: ["daily", 7, null] });
    expect(routine.enabled).toBe(true);
    expect(routine.tags).toEqual(["daily"]);
  });

  /**
   * A routine an agent wrote into the folder. The skill documents the shape,
   * but a model that skipped the schedule or wrote the cron line bare must not
   * take the routines screen down with "cannot read kind of undefined".
   */
  it("gives a hand-written routine a schedule every screen can read", () => {
    expect(settleRoutine({ id: "r", name: "x" }).schedule).toEqual({ kind: "manual" });
    expect(settleRoutine({ id: "r", schedule: "0 9 * * *" }).schedule).toEqual({
      kind: "cron",
      expression: "0 9 * * *",
      humanLabel: "0 9 * * *",
    });
    const kept = { kind: "interval", expression: "PT1H", humanLabel: "Every hour", nextRunAt: null };
    expect(settleRoutine({ id: "r", schedule: kept }).schedule).toEqual(kept);
  });
});

describe("settleMemory", () => {
  it("makes the rendered numbers numbers", () => {
    const memory = settleMemory({ id: "m", useCount: "9", confidence: "180", tags: "solo" });
    expect(memory.useCount).toBe(9);
    expect(memory.confidence).toBe(100);
    expect(memory.tags).toEqual([]);
  });
});

describe("settleThread", () => {
  it("refuses a stored draft", () => {
    // A draft has no file, so a file claiming to be one would hide a real
    // conversation and then be deleted by the next mirror.
    expect(settleThread({ id: "t", draft: true }).draft).toBe(false);
  });

  it("makes the unread badge a number", () => {
    expect(settleThread({ id: "t", unread: "3" }).unread).toBe(3);
    expect(settleThread({ id: "t", unread: null }).unread).toBe(0);
  });

  it("leaves a well-formed list alone", () => {
    const rows = [
      { id: "t", title: "Hi", draft: false, pinned: false, unread: 0, messageCount: 2, agentId: "a" },
    ];
    expect(settleThreads(rows)).toBe(rows);
  });
});

describe("settleUser", () => {
  it("gives an identity file with no preferences the whole default set", () => {
    const user = settleUser({ name: "Elias" });
    expect(user.preferences).toEqual(PREFERENCE_DEFAULTS);
  });

  it("coerces each preference to the type of its own default", () => {
    const user = settleUser({
      name: "Elias",
      preferences: { sendOnEnter: "no", zoom: "120", accent: 7 },
    });
    // A string is not a boolean, so the default stands.
    expect(user.preferences.sendOnEnter).toBe(true);
    expect(user.preferences.zoom).toBe(120);
    expect(user.preferences.accent).toBe(PREFERENCE_DEFAULTS.accent);
  });

  it("keeps a preference this build does not know about", () => {
    const user = settleUser({ preferences: { somethingNewer: "keep me" } });
    expect(user.preferences.somethingNewer).toBe("keep me");
  });

  it("answers a missing document with something the composer can read", () => {
    expect(settleUser(null).preferences.sendOnEnter).toBe(true);
    expect(settleUser("elias").preferences.defaultModelId).toBeNull();
  });

  it("leaves a complete identity alone", () => {
    const user = {
      name: "Elias",
      shortName: "El",
      bio: "",
      preferences: { ...PREFERENCE_DEFAULTS },
    };
    expect(settleUser(user)).toBe(user);
  });
});
