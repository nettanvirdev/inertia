/**
 * The three ways a conversation can run.
 *
 * The picker in the composer offered Chat, Plan and Autonomous from the day it
 * was drawn, and all three did exactly the same thing: the value was local state
 * in the composer and was never sent anywhere. So "Chat" would happily scaffold
 * a Next.js project, and "Plan" would start building instead of planning. A
 * control that does nothing is worse than no control, because the user makes
 * decisions based on it.
 *
 * A mode is two things and only two things:
 *
 *   · **a paragraph in the system prompt**, saying what this turn is for; and
 *   · **a set of tools withheld**, so the paragraph is not merely a request.
 *
 * The second is what makes it real. A model told "do not build anything" while
 * holding `write` and `shell` will, three steps into a promising idea, build
 * something. Taking the tools away is not distrust - it is the difference
 * between an instruction and a guarantee, and it is the only version of this a
 * user can rely on.
 *
 * ## The flow
 *
 * Chat → Plan → the user approves → Autonomous. Plan mode ends by calling
 * `present_plan`, which renders the plan with two buttons: build it, or keep
 * talking. Nothing switches to Autonomous on its own; that is the point of the
 * approval. And the picker is always there, because a person who knows what
 * they want should not have to walk through three modes to get it.
 */

/** Tools that change something outside the conversation. */
const MUTATING = [
  "write",
  "edit",
  "patch",
  "file_copy",
  "file_move",
  "file_folder",
  "file_delete",
  "shell",
  "shell_kill",
  "shell_logs",
  "shell_list",
  "shell_write",
  "worktree_enter",
  "worktree_exit",
  "task",
  // Scheduling a run is starting one; a Chat turn that cannot act now must
  // not be able to act in twenty minutes either.
  "later",
  // Delegation, in both shapes. A spawned run holds full tools of its own, so
  // leaving `spawn` in Chat mode would be a hole straight through the mode: the
  // turn that may not write a file asks a helper to write it.
  "spawn",
  "collect",
  "team",
  "agent_send",
  // Continuing a run is starting one, and stopping one changes what a run that
  // is still going will produce. `wait` is deliberately absent: it changes
  // nothing, it only blocks, so a Chat turn may still wait for work an earlier
  // turn started.
  "followup",
  "interrupt",
  "computer_act",
  "computer_run",
  "computer_write",
  "computer_open",
  "computer_launch",
  "inertia_save",
  "inertia_remove",
  "inertia_set_picture",
  "inertia_set_rules",
  "inertia_connect_app",
];

/** Tools that only look: reading files, searching, seeing a screen. */
const READ_ONLY = [
  "read",
  "lsp",
  "ls",
  "glob",
  "grep",
  "skill",
  "question",
  "todowrite",
  "inertia_list",
  "inertia_get",
  "computer_observe",
  "computer_read",
  "computer_list",
  "computer_page_text",
];

/**
 * The plan, handed to the user for a decision.
 *
 * Only Plan mode has it, and having it is what ends a planning turn: the model
 * cannot approve its own plan, so the only way out of Plan mode is through the
 * user.
 */
const PRESENT_PLAN = "present_plan";

/**
 * The tools that only mean anything in a room.
 *
 * Withheld from every other mode rather than allow-listed into this one, for
 * the same reason as everything else here: a tool nobody withholds is a tool
 * every mode gets, and "hand this conversation to somebody else" offered in a
 * one-agent conversation is an offer that cannot be honoured.
 */
const GROUP_ONLY = ["invite", "handover", "part"];

export const MODES = [
  {
    id: "chat",
    label: "Chat",
    hint: "Talk it through. Nothing gets built.",
    /** Everything mutating is gone; looking things up is not. */
    withhold: [...MUTATING, ...GROUP_ONLY, PRESENT_PLAN],
    prompt: `# This turn is a conversation

You are in Chat mode. The person wants to talk: to ask something, think out
loud, or decide what to do. They have not asked you to build anything, and in
this mode you cannot - the tools that change files, run commands or drive a
computer are not available to you, deliberately.

What you should do is answer well. Read files, search the project, look things
up with whatever connected tools you have, and give a real answer grounded in
what you found rather than a guess. Being unable to write is not being unable to
investigate.

If what they are asking for is a piece of work rather than a question, say so in
a sentence and offer to plan it - switching to Plan mode is one click in the
composer, and you can suggest it. Do not narrate the limitation at length, do
not apologise for it, and never pretend to have done something you cannot do.`,
  },
  {
    id: "plan",
    label: "Plan",
    hint: "Understand the work and write the plan. No changes yet.",
    withhold: [...MUTATING, ...GROUP_ONLY],
    prompt: `# This turn is for planning

You are in Plan mode. The job is to understand what is actually being asked and
to produce a plan good enough to build from - not to build it. The tools that
change anything are withheld for this turn, so the plan is all there is to get
right.

**Investigate before you plan.** Read the files the work touches, look at how
the project is already arranged, check what is really there rather than what a
project of this kind usually has. A plan written without looking is a guess with
numbered steps. Say what you found that changed the shape of the plan.

**Ask when it matters.** If two readings of the request lead to materially
different work, ask - once, with the options named. Do not ask about things you
could have looked up.

**Then call \`present_plan\`.** That is how a plan reaches the person: it renders
with the steps, and they either approve it - which switches this conversation to
Autonomous mode and starts the build - or they keep talking and you revise it.
End your planning turn by calling it. Do not describe a plan in prose and stop,
because then there is nothing for them to approve.

Write the steps as work, not as headings: "Add the price fields to ModelsField
and merge them into modelPrices" is a step; "Pricing" is not. Order them so each
one can actually start when the one before it is done, and name the risky or
uncertain parts rather than smoothing over them.`,
  },
  {
    id: "autonomous",
    label: "Autonomous",
    hint: "Execute the approved plan. Full tools.",
    withhold: [...GROUP_ONLY, PRESENT_PLAN],
    prompt: `# This turn is for building

You are in Autonomous mode, which means you have every tool and the person is
expecting work rather than conversation. If a plan was approved in this
conversation, that plan is the brief - follow it, and keep the task list in step
with where you actually are.

**Finish things.** Do not stop halfway to ask whether to continue; the approval
already answered that. Stop only when the work is done, when you hit something
that genuinely needs a decision only the user can make, or when continuing would
be destructive in a way the plan did not cover.

**Verify what you build.** A file written is not a feature working. Run the
thing, read the output, check the page. Report what you actually observed, and
say plainly when something failed rather than describing the intent as though it
were the result.

**Stay inside the plan.** If you find work the plan did not anticipate, do the
part that the plan needs and tell the user about the rest. Discovering a second
project inside the first one is not permission to build both.`,
  },
  {
    id: "group",
    label: "Agent Group",
    hint: "Several agents in one conversation. Autonomous by default.",
    withhold: [PRESENT_PLAN],
    prompt: `# You are in a group chat

This conversation is a group: several agents and one person, in one thread,
like a messaging group. Everyone reads the same transcript, so you already know
what the others have said, read and decided - there is nothing to fetch and
nobody to ask for a summary. Messages from the person and from the other agents
reach you with the sender's name in front; what you write is yours.

**The person's word is final.** Anything they say outranks anything the agents
have agreed among themselves, including a plan you all just settled on and a
handover somebody just made. If what they ask contradicts the room's plan, the
plan changes. Never tell the person to wait for another agent.

**One message, one voice: yours.** Write only what you have to say, as
yourself, the way one member of a group chat does. Never write a line for a
colleague, never draft what they "would say", never stage a dialogue between
agents inside one reply, and never put your own name or anyone else's name and
a colon in front of a line - the transcript already shows who wrote what.

**Talk like a group, not like a report.** Short messages. Reply to what was
just said. Address a colleague by name when you are answering them. If you
agree, say so in a sentence; if you disagree, say why, once. Add what you
actually have - the thing you know that the others do not, the part of the plan
that will not work - and stop.

**End on the handle of whoever should answer you.** \`@their-handle\`, on the
last line, addressed to the colleague whose reply you actually want. They get
the floor the moment you stop, and they answer you in the same thread. No tool,
no ceremony - the handle is the whole mechanism.

**This is the only thing that keeps the conversation going.** A message that
names nobody ends the round and hands the floor back to the person, however
many questions it contained. "Somebody should check those numbers" has been
asked of nobody and will be answered by nobody; "@statistician - check those
numbers" is a question. If you challenge a colleague, put their handle at the
end, or your challenge is the last thing anyone says.

**Name nobody when you have nothing to put to anyone.** Your part is finished,
or the next move is the person's: end without a handle and it goes back to
them, which is exactly right. Never name a colleague out of politeness. Every
mention is a turn somebody pays for, so mention the one whose answer you need
and not the room.

When the person asks the group an open question, everyone in the room gets one
turn on it, in order, each reading the replies before theirs.

**If you truly have nothing to add, reply with the single word \`pass\`** and
nothing else. Nobody reads it - the transcript just notes that you passed - and
it is far better than a paragraph of agreement.

**Bring somebody new in** by naming them with \`@\` or with \`invite\`. Hand the
conversation over with \`handover\` when it has moved somewhere else for good -
the name and face at the top of the thread change to theirs. Leave with
\`part\` when your part is genuinely finished, not merely because a turn ended.
Every turn costs the person money and attention, so ask for one because the
work needs it, not to be polite.

**Ask the person when the room cannot decide.** A question the agents can settle
between themselves should be settled between themselves; a question about what
the person actually wants should go straight to them. If two of you disagree and
neither can move the other, put both positions to the person in a few lines and
stop.

**Do not repeat each other.** Before you read a file, search a codebase or run a
command, check whether a colleague already did it in this transcript. The whole
point of being in one conversation is that nobody pays for the same work twice.`,
  },
];

/** Modes by id, for the lookups every layer does. */
const BY_ID = new Map(MODES.map((mode) => [mode.id, mode]));

/** The mode a conversation runs in when nobody has said otherwise. */
export const DEFAULT_MODE = "chat";

/**
 * One mode, by id, always. An unknown id is the default rather than a crash:
 * this value comes off a thread record on disk, which a person can edit.
 */
export function modeOf(id) {
  return BY_ID.get(String(id ?? "")) ?? BY_ID.get(DEFAULT_MODE);
}

/** Is this a real mode id? Used where an unknown value should not be stored. */
export function isMode(id) {
  return BY_ID.has(String(id ?? ""));
}

/**
 * The tools this mode keeps, out of the ones the agent's rules already allow.
 *
 * Withholding rather than allow-listing, so a tool added later - an MCP server's,
 * an imported API's - is available in every mode by default. A new way to fetch a
 * document should not be invisible in Chat mode because a list somewhere was not
 * updated; a new way to delete a directory is caught because it goes in MUTATING
 * with the rest.
 */
export function toolsForMode(tools, id) {
  const mode = modeOf(id);
  const withheld = new Set(mode.withhold);
  return (tools ?? []).filter((tool) => !withheld.has(tool?.id ?? tool));
}

/** The paragraph that goes into the system prompt for this mode. */
export function promptForMode(id) {
  return modeOf(id).prompt;
}

export { MUTATING, READ_ONLY, PRESENT_PLAN, GROUP_ONLY };
