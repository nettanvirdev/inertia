import { describe, it, expect } from "vitest";

import { cronToHuman, readSchedule } from "./schedule-check.js";

/**
 * The rule that decides whether a schedule may be saved.
 *
 * The failure this guards against is a screen and a scheduler that disagree
 * about what a valid expression is, so a routine gets saved that never runs.
 * The parser is no longer reachable from here - it is Rust now, in
 * `src-tauri/src/routines.rs` - so the agreement is held in two places instead
 * of one: that module's own tests assert what each of these expressions parses
 * to, and these assert what the screen does with each of those answers. The
 * fixtures below are that parser's real output, not invented ones, and the
 * sentences are the parser's own words.
 */
const PARSED = {
  "0 9 * * 1-5": { at: new Date(Date.now() + 3600_000).toISOString(), reason: null },
  "0 9 * *": { at: null, reason: "A cron expression has five fields." },
  "0 99 * * *": { at: null, reason: "The hour field is out of range." },
  "0 0 30 2 *": { at: null, reason: "That schedule never comes round." },
  PT10S: { at: null, reason: "An interval must be at least a minute." },
  PT1H: { at: new Date(Date.now() + 3600_000).toISOString(), reason: null },
};

const answer = (kind, expression) =>
  readSchedule(
    kind,
    expression === null
      ? { at: null, reason: null }
      : (PARSED[expression] ?? { at: null, reason: "That is not a schedule." })
  );

describe("readSchedule", () => {
  it("accepts an expression the scheduler can fire, and says when", () => {
    const result = answer("cron", "0 9 * * 1-5");
    expect(result.valid).toBe(true);
    expect(Date.parse(result.nextRunAt)).toBeGreaterThan(Date.now());
  });

  it("refuses a typo, and passes on the parser's own reason", () => {
    // Four fields, which is the mistake everyone makes, and the mistake that
    // used to be stored verbatim as a routine that never ran.
    const result = answer("cron", "0 9 * *");
    expect(result.valid).toBe(false);
    expect(result.reason).toMatch(/five fields/i);
  });

  it("refuses a field that is out of range", () => {
    const result = answer("cron", "0 99 * * *");
    expect(result.valid).toBe(false);
    expect(result.reason).toMatch(/hour/i);
  });

  it("refuses a schedule that parses but never comes round", () => {
    // The thirtieth of February. It reads perfectly and fires never, which is
    // exactly as useless as a typo and has to be refused for the same reason.
    const result = answer("cron", "0 0 30 2 *");
    expect(result.valid).toBe(false);
    expect(result.reason).toMatch(/never/i);
  });

  it("refuses an interval the scheduler will not accept", () => {
    expect(answer("interval", "PT10S").valid).toBe(false);
    expect(answer("interval", "PT1H").valid).toBe(true);
  });

  it("asks nothing of the kinds that are not on a clock", () => {
    expect(answer("manual", null).valid).toBe(true);
    expect(readSchedule("trigger", { at: null, reason: null }).valid).toBe(true);
    expect(readSchedule("trigger", { at: null, reason: null }).nextRunAt).toBeNull();
  });

  it("always has a reason to show, even when the parser gave none", () => {
    // A refusal with an empty line under the field is a screen that has
    // stopped working as far as the person using it can tell.
    const result = readSchedule("cron", { at: null, reason: null });
    expect(result.valid).toBe(false);
    expect(result.reason).toBeTruthy();
  });
});

describe("cronToHuman", () => {
  it("reads the shapes this product writes", () => {
    expect(cronToHuman("0 9 * * 1-5")).toBe("Runs Monday to Friday at 09:00.");
    expect(cronToHuman("*/15 * * * *")).toBe("Runs every day every 15 minutes.");
  });

  it("gives up rather than guessing", () => {
    expect(cronToHuman("0 9 * *")).toBeNull();
    expect(cronToHuman("nonsense here at all now")).toBeNull();
  });

  it("does not claim a time zone the scheduler does not use", () => {
    // The scheduler matches cron fields against local time, so the old "UTC"
    // suffix here told people the wrong hour for no reason.
    expect(cronToHuman("0 9 * * *")).not.toMatch(/UTC/);
  });
});
