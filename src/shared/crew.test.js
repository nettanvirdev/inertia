import { describe, expect, it } from "vitest";
import {
  SPAWN_DEFAULTS,
  TEAM_LIMITS,
  canFollowUp,
  activityFor,
  isActive,
  isSettled,
  isWorking,
  mayDelegate,
  spawnPolicy,
  spawnRefusal,
  settledNotice,
} from "./crew.js";

/**
 * Who may build a team.
 *
 * The important cases are the defaults, because they decide what happens to
 * every agent already on disk when this ships: delegating one level stays on,
 * everything else stays off, and nothing anyone configured last month starts
 * behaving differently.
 */
describe("the spawn policy", () => {
  it("leaves an agent with no policy exactly where it was", () => {
    const agent = { name: "Nova" };
    expect(spawnPolicy(agent)).toEqual(SPAWN_DEFAULTS);
    expect(mayDelegate(agent, 0)).toBe(true);
    // Which is the rule that used to be `depth === 0` in the registry.
    expect(mayDelegate(agent, 1)).toBe(false);
  });

  it("lets a nesting agent nest", () => {
    const agent = { name: "Nova", spawn: { subagents: true, recursive: true } };
    expect(mayDelegate(agent, 2)).toBe(true);
  });

  it("refuses a subagent whose parent may not nest, and says what to do instead", () => {
    const refusal = spawnRefusal({ name: "Nova" }, { depth: 1 });
    expect(refusal).toMatch(/not allowed to spawn subagents/i);
    expect(refusal).toMatch(/do this part yourself/i);
  });

  it("refuses an agent that may not delegate at all", () => {
    const refusal = spawnRefusal({ name: "Nova", spawn: { subagents: false } }, {});
    expect(refusal).toMatch(/not allowed to delegate/i);
  });

  it("separates spawning a helper from spawning a configured agent", () => {
    const agent = { name: "Nova", spawn: { subagents: true, agents: false } };
    expect(spawnRefusal(agent, { peer: false })).toBeNull();
    expect(spawnRefusal(agent, { peer: true })).toMatch(/not full agents/i);
  });

  it("enforces a ceiling only when one was set", () => {
    const unlimited = { name: "Nova" };
    expect(spawnRefusal(unlimited, { running: 50 })).toBeNull();

    const capped = { name: "Nova", spawn: { subagents: true, maxConcurrent: 2 } };
    expect(spawnRefusal(capped, { running: 1 })).toBeNull();
    expect(spawnRefusal(capped, { running: 2 })).toMatch(/collect/i);
  });

  it("reads a nonsense ceiling as no ceiling rather than as zero", () => {
    // Zero is how "unlimited" is stored, so a negative or unparseable value
    // must land there too - the alternative is an agent that can spawn nothing
    // because a form once wrote "-1".
    expect(spawnPolicy({ spawn: { maxConcurrent: -4 } }).maxConcurrent).toBe(0);
    expect(spawnPolicy({ spawn: { maxConcurrent: "lots" } }).maxConcurrent).toBe(0);
  });
});

describe("what a status means", () => {
  it("counts queued as active, because it is about to spend money", () => {
    expect(isActive("queued")).toBe(true);
    expect(isActive("running")).toBe(true);
    expect(isSettled("done")).toBe(true);
    expect(isSettled("failed")).toBe(true);
    expect(isSettled("cancelled")).toBe(true);
  });

  it("separates unfinished from working, which paused sits between", () => {
    // Three places depend on paused counting as active: it holds a
    // concurrency slot, `collect` may still wait for it, and a turn must not
    // end while one is sitting there forgotten.
    expect(isActive("paused")).toBe(true);
    expect(isSettled("paused")).toBe(false);
    // But the header dot asks the narrower question, and "one agent working"
    // over a paused run is a lie a glance cannot catch.
    expect(isWorking("paused")).toBe(false);
    expect(isWorking("running")).toBe(true);
  });
});

describe("the activity line", () => {
  it("names what the tool is for, not what it is called", () => {
    expect(activityFor("grep")).toBe("Reading");
    expect(activityFor("edit")).toBe("Writing");
    expect(activityFor("shell_logs")).toBe("Running commands");
    expect(activityFor("computer_act")).toBe("Driving a computer");
    expect(activityFor("spawn")).toBe("Coordinating");
  });

  it("has an answer for a tool it has never heard of", () => {
    // Every MCP server's tools land here, and a blank status line reads as a
    // stuck run.
    expect(activityFor("some_mcp_thing")).toBe("Working");
    expect(activityFor(null)).toBe("Thinking");
  });
});

describe("the conversation-wide limits", () => {
  it("sit above every policy", () => {
    const generous = { name: "Nova", spawn: { subagents: true, agents: true, recursive: true } };
    expect(spawnRefusal(generous, { live: TEAM_LIMITS.live })).toMatch(/in flight/);
    expect(spawnRefusal(generous, { total: TEAM_LIMITS.total })).toMatch(/already started/);
    expect(spawnRefusal(generous, { live: TEAM_LIMITS.live - 1, total: TEAM_LIMITS.total - 1 })).toBeNull();
  });

  it("only a settled run with its conversation kept can take a follow-up", () => {
    expect(canFollowUp("done")).toBe(true);
    expect(canFollowUp("failed")).toBe(true);
    expect(canFollowUp("interrupted")).toBe(true);
    expect(canFollowUp("cancelled")).toBe(false);
    expect(canFollowUp("running")).toBe(false);
    expect(isSettled("interrupted")).toBe(true);
  });
});

/**
 * What a parent is told when a helper finishes while it is still working.
 *
 * The sentence matters more than it looks: it has to say the run is done
 * without reading as an instruction to stop what the parent is doing, or the
 * model abandons its own share of the work to go and collect.
 */
describe("telling a parent a run has settled", () => {
  const run = (id, description, status = "done") => ({ id, description, status });

  it("names the run and how it ended, and says to carry on", () => {
    const note = settledNotice([run("run-1", "write the tests")]);
    expect(note).toMatch(/A run you started has settled/);
    expect(note).toMatch(/run-1 \(write the tests\) finished/);
    expect(note).toMatch(/Carry on with what you were doing/);
    expect(note).toMatch(/collect/);
  });

  it("counts them, and tells a failure from a finish", () => {
    const note = settledNotice([
      run("run-1", "survey the API", "done"),
      run("run-2", "write the tests", "failed"),
      run("run-3", "draft the notes", "cancelled"),
    ]);
    expect(note).toMatch(/3 runs you started have settled/);
    expect(note).toMatch(/run-2 \(write the tests\) failed/);
    expect(note).toMatch(/run-3 \(draft the notes\) was cancelled/);
  });

  it("says nothing when there is nothing to say", () => {
    expect(settledNotice([])).toBe(null);
    expect(settledNotice(null)).toBe(null);
    expect(settledNotice([null, undefined])).toBe(null);
  });
});
