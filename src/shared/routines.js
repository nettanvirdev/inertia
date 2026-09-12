import { DEFAULT_APPROVAL, isApproval } from "./approval.js";
import { isMode } from "./modes.js";

/**
 * What a routine's turn holds, and whether it asks.
 *
 * A routine runs unattended, and a tool call that stops for a person while
 * nobody is there is refused - which is right, and which for a long time was
 * the whole story: every routine ran in Autonomous mode, so it held every
 * tool, but its approval was the conversation default, `ask`, so any call a
 * rule would ask about was refused with a note. What that looked like was a
 * routine that found the file, could not read its size without a shell
 * command, could not send the email, and reported that it would have liked
 * permission. A scheduled job that needs a person is not a scheduled job.
 *
 * So a routine carries the same two dials a conversation does, with different
 * defaults. Mode defaults to Autonomous, because a routine is work approved
 * when it was written. Approval defaults to Never ask, because there is
 * nobody to ask, and a routine the person wanted held back can say so: set it
 * to `ask` and anything a rule would stop for is refused, as before, or to
 * `edits`, and only file changes in the working folder go through. The deny
 * rules hold whatever the dial says, and emptying a folder always asks, which
 * unattended means always refuses - the dial loosens, it never overrides.
 */

export const ROUTINE_DEFAULT_MODE = "autonomous";
export const ROUTINE_DEFAULT_APPROVAL = "auto";

/** The mode a routine's turn runs in: its own if it names a real one. */
export function routineMode(routine) {
  const mode = routine?.mode;
  return isMode(mode) ? mode : ROUTINE_DEFAULT_MODE;
}

/** How much a routine's turn asks: its own if it names a real position. */
export function routineApproval(routine) {
  const approval = routine?.approval;
  return isApproval(approval) ? approval : ROUTINE_DEFAULT_APPROVAL;
}

/** The conversation default, for the one place that compares the two. */
export { DEFAULT_APPROVAL as CONVERSATION_DEFAULT_APPROVAL };
