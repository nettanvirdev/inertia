/**
 * How many tools a turn carries before it has been asked to do anything.
 *
 * Every tool a turn holds is its schema in the request, and the model reads all
 * of them before it writes a word. Measured against a real provider: the core
 * thirteen cost about seven hundred milliseconds to first token, and the full
 * set with a couple of connected apps costs two to three seconds. That is paid
 * on every turn, including the ones that only needed `read`.
 *
 * So there are two answers, and which is right depends on the work:
 *
 *   · **all** - every tool in every request. The model can act immediately and
 *     never has to ask for anything. Right for long autonomous runs, where one
 *     extra second at the top is lost in the noise.
 *   · **on-demand** - the tools that are used constantly are always there, and
 *     the rest wait behind one small tool that loads them by name. Right for
 *     short exchanges, where the delay before the first word is most of what
 *     the person experiences, and for keeping context small.
 *
 * The trade is honest in both directions and is not a matter of taste: on-demand
 * costs one extra round trip on the turn that first needs a loaded family, and
 * a model reaches for what it can see, so a family behind the gate is used less
 * readily than one in front of it. That is why the core is never behind it.
 */

export const TOOL_ACCESS = [
  {
    id: "all",
    label: "Load every tool",
    hint: "The agent can act at once. Slower to start, and a larger context.",
  },
  {
    id: "on-demand",
    label: "Load tools when they are needed",
    hint: "Faster to start and smaller context. One extra step the first time a group is used.",
  },
];

/** What the app does when nobody has said otherwise. */
export const DEFAULT_TOOL_ACCESS = "all";

const BY_ID = new Map(TOOL_ACCESS.map((entry) => [entry.id, entry]));

export function isToolAccess(id) {
  return BY_ID.has(String(id ?? ""));
}

export function toolAccessOf(id) {
  return BY_ID.get(String(id ?? "")) ?? BY_ID.get(DEFAULT_TOOL_ACCESS);
}

/**
 * The tools that are never behind the gate.
 *
 * The test is not "is this important" but "is this used on nearly every turn":
 * a family that is loaded on the first step of every turn costs a round trip
 * for nothing. Reading, searching, writing, running a command, keeping the task
 * list and asking the person a question are that. Everything else is a family
 * some turns never touch.
 *
 * What is left behind the gate is the work a turn either lives in or never
 * touches at all: a browser session, a crew of other agents, and whatever a
 * connected app or MCP server brought with it.
 *
 * `load_tools` is here because a gate the model cannot see is a locked door.
 */
export const ALWAYS_LOADED = new Set([
  "read",
  "write",
  "edit",
  "patch",
  "ls",
  "glob",
  "grep",
  "lsp",
  "shell",
  "todowrite",
  "question",
  "skill",
  "failures",
  "later",
  // Everything the store does. Reading and writing are constant, and
  // forgetting is not: it is here anyway, because a memory that turned out to
  // be wrong is worth deleting the moment it is noticed, and a turn that has
  // to load a tool first will leave it standing.
  "memory_recall",
  "memory_save",
  "memory_forget",
  // Moving files around: the same work as `write` and `edit`, done to whole
  // files instead of their contents, on the same turns.
  "file_copy",
  "file_move",
  "file_folder",
  "file_delete",
  // The other half of `shell`. A command left running in the background is
  // unreadable without these, and the turn that started it needs them.
  "shell_list",
  "shell_logs",
  "shell_write",
  "shell_kill",
  // This workspace's own setup: agents, routines, skills, servers, rules.
  // Inertia configuring itself is close to the core of what it is for.
  "inertia_list",
  "inertia_get",
  "inertia_save",
  "inertia_set_picture",
  "inertia_connect_app",
  "inertia_remove",
  "inertia_set_rules",
  // Handing over the result of the work. A turn that would have to go and
  // fetch this tool before it could show you what it built will simply not
  // show you what it built.
  "present",
  "present_plan",
  // Handing a whole piece of work to another agent, and waiting for it. The
  // one delegation tool that is always there; the asynchronous half is a group.
  "task",
  "worktree_enter",
  "worktree_exit",
  "load_tools",
]);
