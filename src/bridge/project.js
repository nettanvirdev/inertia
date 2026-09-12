import { call } from "./envelope";

/**
 * Setting a project up with the files every coding agent reads.
 *
 * `decide` is deliberately its own call rather than something the window works
 * out from `status` plus a preference: the rule for when to offer belongs in
 * one place, and a second copy of it in the renderer would eventually disagree
 * - which, for a feature whose whole job is not writing into someone's
 * repository uninvited, is the disagreement you least want.
 */
export function projectBridge() {
  return {
    status: (cwd) => call("project_status", { cwd: cwd ?? null }),
    create: (cwd, options) => call("project_create", { cwd: cwd ?? null, options: options ?? {} }),
    decline: (cwd) => call("project_decline", { cwd: cwd ?? null }),
    decide: (cwd, options) => call("project_decide", { cwd: cwd ?? null, options: options ?? {} }),
  };
}
