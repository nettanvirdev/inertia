/**
 * What an agent can do, named once.
 *
 * A permission rule is written about a key, and a key has to mean the same
 * thing in three places at once: the settings screen where a person chooses
 * `ask` or `deny`, the registry that decides which tools a model is even told
 * about, and the tool itself when it stops mid-call to ask. One list, imported
 * by all three, is the only way those three stay in agreement.
 *
 * The keys are deliberately coarser than the tools. `write` and `edit` both
 * ask under `edit`, because "may this agent change my files" is the question a
 * person actually has an opinion about, and splitting it into two switches
 * produces a screen where the answer is always the same on both.
 *
 * Tools that arrive at runtime - from an MCP server, an OpenAPI import, a
 * Composio app - are not in this list by name. They cannot be: their names are
 * not knowable until the thing is connected. Each source gets one key here
 * instead, and the tool carries the identity of what it came from in the
 * target, so a rule can name a server or an app without knowing in advance what
 * that server decided to call its tools.
 */

export const GROUPS = [
  { id: "files", label: "Files", description: "Reading and changing files on this machine." },
  { id: "system", label: "System", description: "Running commands and reaching outside the project." },
  { id: "agents", label: "Agents and skills", description: "Delegating work and loading instructions." },
  { id: "plugins", label: "Connected tools", description: "MCP servers, APIs and Composio apps." },
];

/**
 * `danger` drives how loudly the prompt is drawn and what the sensible default
 * is. It is a statement about what the tool can do at its worst, not about how
 * often it does it: `shell` is marked high because one command can be
 * irreversible, even though almost every command is `git status`.
 */
export const TOOLS = [
  {
    key: "read",
    label: "Read and search files",
    description:
      "Open a file or list a directory and see the contents, find files by name, and " +
      "search inside them.",
    group: "files",
    icon: "FileText",
    danger: "low",
    fallback: "allow",
    // `present` rides here rather than under a key of its own: it opens
    // nothing the reader could not open themselves and changes nothing, so
    // asking about it would be a card with no decision in it. `glob` and `grep`
    // ask under this key too - finding a file is looking at it - and a switch
    // of their own here would be one the tools never consult.
    tools: ["read", "ls", "lsp", "present", "glob", "grep"],
  },
  {
    key: "edit",
    label: "Change files",
    description:
      "Create, overwrite or edit a file, and move, copy or delete one. Shows a diff " +
      "before it asks.",
    group: "files",
    icon: "FilePen",
    danger: "high",
    fallback: "ask",
    tools: ["write", "edit", "patch", "file_copy", "file_move", "file_folder", "file_delete"],
  },
  {
    key: "shell",
    label: "Run commands",
    description: "Run a command in a terminal. Rules can name specific commands.",
    group: "system",
    icon: "SquareTerminal",
    danger: "high",
    fallback: "ask",
    tools: ["shell", "shell_write", "worktree_enter", "worktree_exit"],
    // The one key where per-pattern rules are the point rather than a refinement,
    // so the settings screen offers these as a starting point.
    suggestions: [
      { pattern: "git *", action: "allow", label: "Git, except the ones below" },
      { pattern: "git push *", action: "ask", label: "Pushing" },
      { pattern: "npm run *", action: "allow", label: "Package scripts" },
      { pattern: "rm *", action: "ask", label: "Deleting" },
      { pattern: "rm -rf /*", action: "deny", label: "Deleting from the root" },
      { pattern: "curl * | sh", action: "deny", label: "Piping the internet into a shell" },
    ],
  },
  {
    key: "computer",
    label: "Use its own computer",
    description:
      "Run commands and change files on the sandbox assigned to this agent - a different " +
      "machine from yours.",
    group: "system",
    icon: "Monitor",
    danger: "medium",
    fallback: "ask",
    // Nine tools, one key. The split exists so the model picks the right one;
    // the user should not pay for it with nine rules to write.
    tools: [
      "computer_observe",
      "computer_act",
      "computer_open",
      "computer_launch",
      "computer_page_text",
      "computer_run",
      "computer_list",
      "computer_read",
      "computer_write",
    ],
    // Lower danger than `shell` and it is not a mistake: the whole point of a
    // sandbox is that the worst case is losing the sandbox. The rule still
    // defaults to asking, because a machine that costs money to run is a
    // machine the user should know is being used.
    suggestions: [
      { pattern: "observe", action: "allow", label: "Looking at its screen" },
      { pattern: "act", action: "allow", label: "Clicking and typing on it" },
      { pattern: "page_text", action: "allow", label: "Reading a page on it" },
      { pattern: "list", action: "allow", label: "Listing its files" },
      { pattern: "read", action: "allow", label: "Reading its files" },
      { pattern: "*", action: "ask", label: "Running anything" },
    ],
  },
  {
    key: "browser",
    label: "Use the browser pane",
    description:
      "Open pages in the browser beside the conversation, read them, click and type in them, and " +
      "read back the console and network log. This is your own browser on this machine, with your " +
      "logged-in sessions and your localhost.",
    group: "system",
    icon: "Globe",
    // Higher than the sandboxed computer and lower than the shell, and the
    // reason is the session. This browser carries the person's own cookies, so
    // a page it opens may be one they are signed in to - which is exactly what
    // makes it useful for looking at a dev server, and exactly what makes
    // "click anything on any site" a thing worth being asked about.
    danger: "medium",
    fallback: "ask",
    tools: [
      "browser_navigate",
      "browser_read_page",
      "browser_read_text",
      "browser_click",
      "browser_type",
      "browser_press",
      "browser_evaluate",
      "browser_console",
      "browser_network",
    ],
    suggestions: [
      { pattern: "http://localhost*", action: "allow", label: "Anything on localhost" },
      { pattern: "http://127.0.0.1*", action: "allow", label: "Anything on 127.0.0.1" },
      { pattern: "the open page", action: "allow", label: "Reading and driving the page already open" },
      { pattern: "*", action: "ask", label: "Opening anything else" },
    ],
  },
  {
    key: "terminal_read",
    label: "Read your terminal",
    description:
      "Read the terminal pane beside the conversation - what you typed and what it printed. It " +
      "runs nothing.",
    group: "system",
    icon: "SquareTerminal",
    // The lowest there is, and it earns it: this tool cannot change anything.
    // What it can do is read output, and output from a build is not private in
    // any sense the person has not already accepted by having the pane open
    // next to a conversation.
    danger: "low",
    fallback: "allow",
    tools: ["terminal_read"],
  },
  {
    key: "delete_everything",
    label: "Delete everything",
    description:
      "Delete a folder and everything in it, empty a folder, or throw away work that " +
      "was never saved. Always asks, even inside the working folder, and there is no " +
      "way to turn it off.",
    group: "system",
    icon: "Trash2",
    danger: "high",
    fallback: "ask",
    tools: [],
    // No suggestions on purpose. Every other capability offers a shortcut to
    // stop being asked; this one is the exception, because a sweep is the one
    // action with nothing left to inspect afterwards.
  },
  {
    key: "external_directory",
    label: "Reach outside the project",
    description: "Touch a directory that is not the working folder.",
    group: "system",
    icon: "FolderOpen",
    danger: "high",
    fallback: "ask",
    tools: [],
  },
  {
    key: "task",
    label: "Delegate to a subagent",
    description: "Hand a piece of work to another agent and wait for its answer.",
    group: "agents",
    icon: "Users",
    danger: "medium",
    fallback: "allow",
    tools: ["task"],
  },
  {
    key: "skill",
    label: "Load a skill",
    description: "Pull a skill's instructions into the conversation.",
    group: "agents",
    icon: "Sparkles",
    danger: "low",
    fallback: "allow",
    tools: ["skill"],
  },
  {
    key: "memory",
    label: "Remember and recall",
    description:
      "Read what it already knows from earlier conversations, write down something worth " +
      "keeping, and forget something it was told.",
    group: "agents",
    icon: "Brain",
    danger: "low",
    fallback: "allow",
    tools: ["memory_recall", "memory_save", "memory_forget"],
    // The targets are what the tools ask about: `recall`, `remember <title>`
    // and `forget <id>`.
    suggestions: [
      { pattern: "recall", action: "allow", label: "Reading what it knows" },
      { pattern: "remember *", action: "allow", label: "Writing something down" },
      { pattern: "forget *", action: "ask", label: "Forgetting something" },
    ],
  },
  {
    key: "load_tools",
    label: "Load more of its own tools",
    description:
      "When the workspace is set to load tools only when they are needed, this is how an " +
      "agent asks for the rest of them. It grants nothing on its own: a tool it loads is " +
      "still asked about under its own rule when it runs.",
    group: "agents",
    icon: "PackagePlus",
    danger: "low",
    fallback: "allow",
    tools: ["load_tools"],
  },
  {
    key: "todowrite",
    label: "Keep a task list",
    description: "Track the steps of a long piece of work.",
    group: "agents",
    icon: "ListChecks",
    danger: "low",
    fallback: "allow",
    tools: ["todowrite"],
  },
  {
    key: "question",
    label: "Ask you a question",
    description: "Stop and ask before choosing between options.",
    group: "agents",
    icon: "MessageCircle",
    danger: "low",
    fallback: "allow",
    tools: ["question"],
  },
  {
    key: "doom_loop",
    label: "Repeat the same call",
    description:
      "An agent has made the same tool call, with the same arguments, three times in a " +
      "row. Asking stops a loop before it costs forty steps; allowing lets it through.",
    group: "agents",
    icon: "RotateCcw",
    danger: "medium",
    fallback: "ask",
    tools: [],
  },
  {
    key: "failures",
    label: "Read the failure log",
    description:
      "Look up what has gone wrong before: failed tool calls, refused permissions, " +
      "provider errors, routine and subagent failures. Read-only.",
    group: "agents",
    icon: "TriangleAlert",
    danger: "low",
    fallback: "allow",
    tools: ["failures"],
  },
  {
    key: "inertia",
    label: "Set up Inertia",
    description:
      "Create or change agents, routines, skills, MCP servers, API imports and memories, " +
      "give an agent a picture, and connect apps. Every change appears on screen at once.",
    group: "agents",
    icon: "Settings",
    danger: "medium",
    fallback: "allow",
    tools: [
      "inertia_list",
      "inertia_get",
      "inertia_save",
      "inertia_set_picture",
      "inertia_connect_app",
    ],
  },
  {
    key: "inertia_guarded",
    label: "Remove from Inertia, or change its rules",
    description:
      "Delete an agent, routine, skill, server, import or connected app - one at a time, " +
      "never a sweep - rewrite permission rules, or change the program an MCP server runs " +
      "or the address it connects to. Asks every time by default.",
    group: "agents",
    icon: "Trash2",
    danger: "high",
    fallback: "ask",
    tools: ["inertia_remove", "inertia_set_rules"],
  },
  {
    key: "mcp",
    label: "MCP servers",
    description: "Tools from connected Model Context Protocol servers.",
    group: "plugins",
    icon: "Plug",
    danger: "medium",
    fallback: "ask",
    tools: [],
    dynamic: true,
  },
  {
    key: "openapi",
    label: "API operations",
    description: "Calls to APIs you imported from an OpenAPI spec.",
    group: "plugins",
    icon: "Globe",
    danger: "medium",
    fallback: "ask",
    tools: [],
    dynamic: true,
  },
  {
    key: "composio",
    label: "Composio apps",
    description: "Actions in the apps you connected through Composio.",
    group: "plugins",
    icon: "Blocks",
    danger: "medium",
    fallback: "ask",
    tools: [],
    dynamic: true,
  },
];

export const TOOL_KEYS = TOOLS.map((tool) => tool.key);

export function toolByKey(key) {
  return TOOLS.find((tool) => tool.key === key) ?? null;
}

/**
 * The key a tool id asks under.
 *
 * Falls back to the id itself, which is only ever reached by a tool that names
 * no key of its own. The runtime sources all name one - `mcp`, `openapi`,
 * `composio` - so their tools are looked up by `tool.permission.key` and never
 * get here.
 */
export function keyForTool(id) {
  const owner = TOOLS.find((tool) => tool.tools.includes(id));
  return owner ? owner.key : id;
}

/**
 * The starting ruleset for a new agent.
 *
 * Read and search are allowed because an agent that has to ask before looking
 * at anything is useless, and looking is not destructive. Everything that
 * changes the world asks. This is a deliberate, opinionated default and the
 * user can move any of it.
 */
export function defaultRules() {
  return TOOLS.filter((tool) => !tool.dynamic).map((tool) => ({
    tool: tool.key,
    pattern: "*",
    action: tool.fallback,
  }));
}

export function groupedTools() {
  return GROUPS.map((group) => ({
    ...group,
    tools: TOOLS.filter((tool) => tool.group === group.id),
  })).filter((group) => group.tools.length);
}
