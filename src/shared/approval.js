/**
 * How much a conversation asks.
 *
 * The permission rules say, per tool and per pattern, whether a call is
 * allowed, asked about or refused. They are the right place for a standing
 * opinion - "git push always asks", "never pipe the internet into a shell" -
 * and the wrong place for a mood. A person who has watched an agent make six
 * good edits and wants the next twenty to go through does not want to open
 * Settings and rewrite the edit rule; they want a dial on the conversation
 * that says "stop asking me about this" and, when the work is done, turns
 * back.
 *
 * So this is the dial, and it is deliberately narrower than the rules. It
 * can only turn an `ask` into an `allow`. A rule that says `deny` still
 * denies whatever the dial says, because a deny rule is a decision the person
 * made in advance about something they did not want to be tempted by; and a
 * handful of keys ask whatever the dial says, because they are the ones with
 * nothing left to inspect afterwards.
 *
 * Three positions, the same three Claude Code settled on and Codex has under
 * other names:
 *
 *   ask     the rules as written
 *   edits   file changes go through; commands and everything else still ask
 *   auto    nothing asks, except what may never be waved through
 *
 * Distinct from the conversation mode on purpose. Chat, Plan and Autonomous
 * decide which tools a turn *holds*; this decides whether the tools it holds
 * stop to ask. Autonomous mode with "ask" is an agent that can do anything
 * and checks first; Chat mode with "auto" is an agent that cannot change
 * anything and would not ask if it could. Both are coherent, and neither is
 * expressible if the two dials are one.
 */

export const APPROVALS = [
  {
    id: "ask",
    label: "Ask",
    hint: "Your permission rules, as written.",
  },
  {
    id: "edits",
    label: "Accept edits",
    hint: "File changes in the working folder go through. Commands and everything else still ask.",
  },
  {
    id: "auto",
    label: "Never ask",
    hint: "Anything a rule would ask about is allowed. Deny rules still hold, and emptying a folder always asks.",
  },
];

export const DEFAULT_APPROVAL = "ask";

/**
 * What "Accept edits" waves through.
 *
 * Only the edit key. Reading and searching are allowed by the default rules
 * already, and a person who tightened those to `ask` did it on purpose. The
 * `external_directory` key is not here either: an edit outside the working
 * folder is the one edit that "accept edits in this project" does not cover.
 */
export const EDIT_KEYS = ["edit"];

/**
 * What no position of the dial may wave through.
 *
 * `delete_everything` says on its own catalog card that there is no way to
 * turn it off, and the dial is not going to be the way. It is the one action
 * with nothing left to look at afterwards.
 */
export const ALWAYS_ASK = ["delete_everything"];

export function isApproval(id) {
  return APPROVALS.some((approval) => approval.id === id);
}

export function approvalOf(id) {
  return APPROVALS.find((approval) => approval.id === id) ?? APPROVALS[0];
}

/**
 * What the dial does with a verdict of `ask`.
 *
 * Returns `allow`, `ask` or `deny`. The one `deny` is the loop guard under
 * "Never ask": a fourth identical call is a question - "did you mean to?" -
 * and with nobody to put it to, the honest answer is to stop the loop rather
 * than let it run to forty steps. That is the whole point of the guard, and
 * a dial that silenced it would be worse than no guard.
 */
export function resolveAsk(approval, key) {
  if (ALWAYS_ASK.includes(key)) return "ask";
  if (approval === "auto") return key === "doom_loop" ? "deny" : "allow";
  if (approval === "edits" && EDIT_KEYS.includes(key)) return "allow";
  return "ask";
}

/**
 * What the model is told about the dial.
 *
 * It matters for two reasons. Under "ask", a model that knows a call will
 * stop for a person writes the call so the person can judge it - a diff, not
 * a rewrite. Under "auto", a model that knows nobody is checking is the one
 * that has to check; Codex tells its model the approval policy for the same
 * reason.
 */
export function promptForApproval(approval) {
  switch (approval) {
    case "auto":
      return (
        "Never ask: tool calls the user's rules would stop to ask about are allowed without " +
        "anyone looking. Nobody is checking each step, so you are. Prefer the reversible " +
        "version of an action, verify what you change, and say plainly in your reply what " +
        "you did that would normally have been asked about."
      );
    case "edits":
      return (
        "Accept edits: changes to files in the working folder go through without asking; " +
        "commands, anything outside the folder, and everything else still stop for the user."
      );
    default:
      return "Ask: the user's permission rules apply as written, and a call they would ask about stops until they answer.";
  }
}
