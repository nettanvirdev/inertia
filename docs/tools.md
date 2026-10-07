# Tools reference

Every tool an agent can call, what it does, its main inputs, and the
permission key it asks under. Tool descriptions the model reads live next to
each tool's code; this page summarises them.

Where the tools come from:

| Source | Code |
|---|---|
| Built-in tools | `src-tauri/crates/inertia-tools/src/builtin/` |
| Memory tools | `src-tauri/crates/inertia-memory/src/tools.rs` |
| App-level tools (sub-agents, routines, setup, computers, ...) | `src-tauri/src/*.rs` |
| MCP, OpenAPI and Composio tools | `src-tauri/crates/inertia-{mcp,openapi,composio}/src/provider.rs` |

All of them, whatever the source, go through the same pipeline in the tool
registry: parse and repair the arguments, validate against the tool's JSON
Schema, ask permission, run, and cap the output so one call cannot flood the
context window. A call that fails validation never runs and never asks.

## Permissions

### Keys and targets

Every call asks under a **key** and names a **target**. Several tools share a
key when a person would think of them as one capability; `write`, `edit` and
`patch` all ask under `edit`, for example. The target is what a rule is
matched against: the path for a file tool, the full command for `shell`, the
URL for the browser.

### Rules

A rule is `{ "tool": <key glob>, "pattern": <target glob>, "action": "allow" | "ask" | "deny" }`.
Globs support only `*` (any run of characters, including newlines) and `?`
(one character); everything else is literal. Examples:

```json
{ "tool": "shell", "pattern": "git *",       "action": "allow" }
{ "tool": "shell", "pattern": "git push *",  "action": "ask" }
{ "tool": "shell", "pattern": "rm -rf /*",   "action": "deny" }
{ "tool": "mcp",   "pattern": "github_*",    "action": "allow" }
```

How a call is decided:

- The **most specific** matching rule wins, not the first. A rule naming the
  key exactly beats one that matched it with a wildcard; an exact pattern
  beats any wildcard pattern; among wildcard patterns, more literal text wins.
  Ties go to the earlier rule.
- **No matching rule means `ask`.**
- A `deny` is final for that call. The person is never shown a card whose
  "Allow" button could contradict it.
- A tool denied for **every** target (`pattern: "*"`, `action: "deny"`) is not
  offered to the model at all. A tool denied only for some targets stays
  visible.

### Scopes

Rules live in `settings/permissions.json`:

```json
{
  "workspace": [ { "tool": "read", "pattern": "*", "action": "allow" } ],
  "agents": {
    "agent-reviewer": [ { "tool": "shell", "pattern": "*", "action": "deny" } ]
  }
}
```

`workspace` rules apply to every agent. `agents.<id>` rules are layered on top
for that agent; a rule with the same key and pattern replaces the workspace
one, so an agent can be tightened or loosened. Edit them in **Settings >
Permissions** or on an agent's permission matrix.

The default ruleset offered in Settings allows reading, searching, skills,
memory (including `memory_forget`), the task list, questions, `task`, the
failure log, `inertia` setup and reading the terminal, and asks for everything that changes something: `edit`,
`shell`, `computer`, `browser`, `external_directory`, `delete_everything`,
`inertia_guarded`, `mcp`, `openapi` and `composio`. The catalogue of keys and
their defaults is `src/shared/tools.js`.

### Asking and "Always allow"

When a call needs asking, a card appears in the conversation showing the key
and the exact target. The answers are **Allow** (this call), **Always allow**
(writes an allow rule into the workspace rules), and **Deny**. "Always allow"
is only offered when there is something sensible to remember. Dismissing the
card, cancelling the turn or closing the window counts as **Deny**.

"Always allow" remembers a generalised pattern, not always the literal target:

| Key | What "Always allow" remembers |
|---|---|
| `read` | everything (`*`) |
| `edit` | that exact path |
| `shell` | the command's recognisable prefix plus ` *`, e.g. `git status -s` becomes `git status *` (which also matches bare `git status`) and `npm run build -- --watch` becomes `npm run build *` |
| `browser`, `task`, `skill` (per skill name), `todowrite`, `question`, `terminal_read`, `failures` | everything for that key (or that skill) |
| `mcp` | that one MCP tool |
| `delete_everything`, `memory_forget`, `inertia_guarded` MCP changes, a shell command naming `secrets/` | never offered |

For `shell`, a chained command (`a && b`, `a; b`, `a | b`) is allowed only if
every segment is allowed, and a deny on any segment denies the whole line, so a
rule allowing `git *` does not allow `git status && rm -rf build`. Command
substitution (`$(...)`, backticks), process substitution (`<(...)`), more than
one line, an unclosed quote, or output redirected into a real file always asks
unless the shell is allowed outright (`shell` with pattern `*`). A command that
names the workspace's `secrets/` folder always shows a card, with no "Always
allow". Parsing is a heuristic that errs towards asking.

### Modes

Each conversation has a mode, chosen in the composer. A mode withholds tools:
the model is not told about them at all, so it cannot try them.

| Mode | Withholds |
|---|---|
| Chat | everything that changes something (below), `present_plan`, and the group tools |
| Plan | everything that changes something, and the group tools. Ends by calling `present_plan`. |
| Autonomous | `present_plan` and the group tools |
| Agent Group | `present_plan` only |

"Everything that changes something" is: `write`, `edit`, `patch`,
`file_copy`, `file_move`, `file_folder`, `file_delete`, `shell`,
`shell_kill`, `shell_logs`, `shell_list`, `shell_write`, `worktree_enter`,
`worktree_exit`, `task`, `later`, `spawn`, `collect`, `team`, `agent_send`,
`followup`, `interrupt`, `computer_act`, `computer_run`, `computer_write`,
`computer_open`, `computer_launch`, `inertia_save`, `inertia_remove`,
`inertia_set_picture`, `inertia_set_rules`, `inertia_connect_app`. The list is
`MUTATING` in `src-tauri/crates/inertia-agent/src/prompt.rs` (mirrored in
`src/shared/modes.js`).

### Keys that are always separate

- **`delete_everything`**: deleting a folder recursively, or a shell command
  that looks like it empties a folder (`rm -rf <dir>`, `Remove-Item -Recurse`,
  ...). Asked in addition to `edit`/`shell`, and has no "Always allow".
- **`external_directory`**: a file tool, a shell command or `present`
  reaching a path outside the working folder and outside `<workspace>/files/`. Asked in addition to the tool's
  own key, with the folder (`<dir>/*`) as the target.
- **`doom_loop`**: the same tool call with the same arguments three times in a
  row. Asking stops a loop.

Dangerous-looking shell commands (`rm -rf /`, `mkfs`, `curl ... | sh`,
force-pushing to `main`, `DROP DATABASE`, ...) are flagged on the approval card.
The flag is advisory; it never allows anything by itself.

## Files

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `read` | Read a file with line numbers (up to 2000 lines per call), or list a directory. Image files are returned as images. | `filePath`, `offset`, `limit` | `read` |
| `write` | Create or replace a file, creating parent folders. | `filePath`, `content` | `edit` |
| `edit` | Replace exact text in a file read earlier in the conversation. Must match one place unless `replaceAll`. | `filePath`, `oldString`, `newString`, `replaceAll` | `edit` |
| `patch` | Apply a unified diff across one or more files, all or nothing. Can create files; cannot delete them. Each file must have been read first. | `diff` | `edit` (per file) |
| `file_copy` | Copy a file or folder. Refuses to overwrite unless asked. | `source`, `destination`, `overwrite` | `edit` |
| `file_move` | Move or rename a file or folder. | `source`, `destination`, `overwrite` | `edit` |
| `file_folder` | Create a folder and its parents. | `path` | `edit` |
| `file_delete` | Delete a file, or a folder with `recursive: true`. Permanent (no recycle bin). | `path`, `recursive` | `edit`, plus `delete_everything` for a folder |
| `ls` | List a directory as a tree, skipping `node_modules`, `.git` and similar. | `path`, `depth`, `all` | `read` |
| `present` | Show the person the finished files of the work so they can open them. | `paths`, `note` | `read` (`external_directory` for files outside the folder) |

`edit` and `patch` refuse a file that changed on disk since it was read.
Relative paths resolve against the conversation's working folder. A path
outside the working folder (and outside `<workspace>/files/`) also asks
`external_directory`, once per folder. Some places are refused to every file
tool (`read`, `write`, `edit`, `patch`, `glob`, `grep`, `ls`, `lsp`, `present`,
the `file_*` tools, and `shell`'s `workdir`) whatever the rules say:
`<workspace>/secrets/`, `<workspace>/settings/permissions.json`,
`<workspace>/hooks/`, and any project's `.inertia/hooks.json`. Containment is
checked on canonical paths, so symlinks and junctions cannot step around it.

## Search and code intelligence

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `glob` | Find files by name pattern. Respects `.gitignore`. | `pattern`, `path` | `read` |
| `grep` | Search file contents with a regular expression. Respects `.gitignore`. | `pattern`, `path`, `include`, `ignoreCase` | `read` |
| `lsp` | Ask a language server: `definition`, `type_definition`, `implementation`, `references`, `hover`, `symbols`, `workspace_symbols`. Positions are 1-based. | `operation`, `filePath`, `line`, `character`, `symbol` | `read` |

`lsp` uses servers found on your `PATH`: `typescript-language-server`,
`pyright-langserver`, `ruff`, `gopls`, `rust-analyzer`. With none installed for
a language it says so and the model falls back to `grep`.

## Shell

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `shell` | Run a command on your machine. Working folder persists between calls; shell state does not. Stdin is closed. `background: true` starts a long-running process and returns its pid. | `command`, `workdir`, `timeout`, `background` | `shell` (target: the command) |
| `shell_list` | List background processes this conversation started. | none | `shell` |
| `shell_logs` | Read the recent output of a background process. | `pid` | `shell` |
| `shell_kill` | Stop a background process and its children. | `pid` | `shell` |
| `shell_write` | Type into a background process's stdin. | `pid`, `input`, `newline`, `end` | `shell` |
| `terminal_read` | Read the terminal tabs beside the conversation: what you typed and what it printed. Runs nothing. | `chars` | `terminal_read` |
| `worktree_enter` | Create (or reuse) a git worktree at `.inertia/worktrees/<name>` on branch `inertia/<name>` and move the conversation into it. | `name`, `base` | `shell` |
| `worktree_exit` | Return to the main checkout; optionally remove the worktree folder. The branch is kept. | `remove`, `force` | `shell` |

The shell is PowerShell on Windows (`pwsh` if installed, otherwise Windows
PowerShell 5.1) and `$SHELL` (default `/bin/bash`) elsewhere. Commands run with
your user's privileges; see [security.md](security.md).

## Planning and conversation

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `todowrite` | Write the task list for the current work (shown to you). | `todos` | `todowrite` |
| `present_plan` | End a Plan-mode turn by showing a plan with Build / Keep talking buttons. Plan mode only. | `title`, `summary`, `steps`, `risks` | `todowrite` |
| `question` | Ask you a question and wait for the answer without ending the turn. | `question`, `options` | `question` |
| `skill` | Load a skill's instructions and a listing of its folder. | `name` | `skill` (target: the skill name) |
| `failures` | Read the log of earlier failures: failed calls, refusals, provider errors, crashed routines. Read-only. | `query`, `kind`, `tool`, `agent`, `days`, `limit`, `verbose` | `failures` |
| `load_tools` | Only when **Settings > General > Tool access** is "Load tools when they are needed": loads a group of tools for the rest of the turn. Grants nothing; each tool still asks under its own key. | `groups` | `load_tools` |

## Memory

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `memory_recall` | Return the full text of memories matching a query. | `query`, `limit` | `memory` (target `recall`) |
| `memory_save` | Save one durable fact. | `title`, `body`, `scope` (`global` or `project`), `kind`, `tags` | `memory` (target `remember <title>`) |
| `memory_forget` | Delete one memory by id. | `id` | `memory` (target `forget <id>`), no "Always allow". The default `memory` rule allows it; add `memory` / `forget *` / `ask` to be asked. |

## Sub-agents and the crew

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `task` | Hand a self-contained job to another agent (or a temporary helper) and wait for its report. Sub-agents cannot call `task` themselves. | `description`, `prompt`, `subagent_type`, `task_id` | `task` |
| `spawn` | Start a background run and return its id immediately. | `description`, `prompt`, `agent`, `role`, `model`, `isolation`, `share_context` | `task` |
| `collect` | Wait for spawned runs and read their results. | `run_ids`, `timeout_seconds` | `task` |
| `wait` | Block until a run finishes, a message arrives, or you say something. | `run_ids`, `timeout_seconds` | `task` |
| `team` | What every spawned run is doing, plus messages sent to you. Never waits. | none | `task` |
| `agent_send` | Send a message to another run's inbox. | `to`, `message` | `task` |
| `interrupt` | Stop a run mid-turn but keep its transcript. | `run_id`, `reason` | `task` |
| `followup` | Give a settled run a new brief with its context intact. | `run_id`, `prompt` | `task` |

Per-agent delegation policy decides whether an agent may spawn at all, how
deep, and how many at once. Above that there is a hard ceiling of 20 live runs
and 100 runs in total per conversation.

## Group conversations

Offered only in Agent Group mode.

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `invite` | Bring another agent into the conversation; they answer next. Writing `@handle` does the same. | `agent`, `why` | `task` |
| `handover` | Give the conversation to another agent. | `agent`, `note` | `task` |
| `part` | Leave the conversation and hand it back. | `summary`, `to` | `task` |

## Routines

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `later` | Schedule a one-off routine that runs a brief at a later time, in its own conversation under Routines. | `brief`, `in_minutes` or `at`, `name` | `inertia` (target `routine`) |

## Setting up Inertia

These let an agent manage the workspace itself, through the same validation
the screens use. Every change appears in the window immediately.
Conversations, history and secrets are not reachable through them.

| Tool | Does | Key inputs | Key |
|---|---|---|---|
| `inertia_list` | List agents, routines, skills, memories, computers, MCP servers, API imports and connected apps. | `kind` | `inertia` |
| `inertia_get` | Read one of those in full. | `kind`, `id` | `inertia` |
| `inertia_save` | Create or update an agent, routine, skill, memory, MCP server or API import. Only the fields sent are changed. | `kind`, `id`, `fields` | `inertia`; creating an MCP server or changing what it runs or connects to (`type`, `command`, `args`, `cwd`, `env`, `url`, `headers`) asks under `inertia_guarded`, showing the full command line, with no "Always allow" |
| `inertia_set_picture` | Set or clear an agent's picture from a URL or a local image. | `agentId`, `url`, `path`, `clear` | `inertia` |
| `inertia_connect_app` | Search the Composio catalogue, start a connection (returns a sign-in link), or check its status. | `toolkit`, `search`, `status` | `inertia` |
| `inertia_remove` | Delete one agent, routine, skill, memory, MCP server or API import, or disconnect one app. One item per call. | `kind`, `id` | `inertia_guarded` |
| `inertia_set_rules` | Change permission rules for an agent or the workspace. The card lists every rule. | `agentId`, `rules` | `inertia_guarded` |

Records marked `protected` (the app's seeded ones) cannot be changed or removed
through these tools.

## The browser pane

The browser pane is a webview inside the Inertia window, on your machine, with
your own cookies and access to `localhost`. You watch it as the agent drives
it. All of these share the `browser` key; the target is the URL (or `the open
page`).

| Tool | Does | Key inputs |
|---|---|---|
| `browser_navigate` | Open a URL and return the page outline. | `url` |
| `browser_read_page` | The page as an accessibility-style outline, with a `ref_N` per interactive element. | |
| `browser_read_text` | The readable text of the page. | `maxChars` |
| `browser_click` | Click an element by `ref`. | `ref` |
| `browser_type` | Type text, optionally into an element by `ref`. | `text`, `ref` |
| `browser_press` | Press a key or chord (`Enter`, `Control+a`). | `keys` |
| `browser_evaluate` | Run JavaScript in the page and return the result, for inspection. | `code` |
| `browser_console` | Console output since the last navigation. | `onlyErrors`, `pattern` |
| `browser_network` | Requests since the last navigation, with status and timing. | `onlyFailed`, `urlPattern` |

Input is dispatched as DOM events from inside the page (Tauri webviews cannot
inject OS input), which covers listener-based UIs but not behaviour the browser
only performs for trusted events. There is no screenshot tool for the pane.

## Computers

Offered only to an agent with a computer assigned. All share the `computer`
key, with a target naming the action (`observe`, `act`, `list`, `read`,
`write`, `page_text`), the URL or file for `computer_open`, the application for
`computer_launch`, and the command for `computer_run`. See
[computers.md](computers.md).

| Tool | Does | Key inputs |
|---|---|---|
| `computer_observe` | Screenshot plus screen size, active window and cursor position. | none |
| `computer_act` | A list of pointer and keyboard actions run in one go (`click`, `move`, `down`, `up`, `type`, `key`, `scroll`, `wait`), then optionally a screenshot. | `actions`, `settle_ms`, `observe` |
| `computer_open` | Open a file or URL in the machine's default application. | `target` |
| `computer_launch` | Start an application (`browser` or a command name). | `application`, `uri` |
| `computer_page_text` | Text of the page the machine's browser is showing. | none |
| `computer_run` | Run a shell command on the machine. | `command`, `cwd`, `timeout` |
| `computer_list` | List a directory on the machine. | `path` |
| `computer_read` | Read a text file on the machine. | `path` |
| `computer_write` | Create or overwrite a text file on the machine. | `path`, `content` |

## MCP, OpenAPI and Composio tools

Tools from integrations appear alongside the built-ins and go through the same
pipeline. See [integrations.md](integrations.md).

| Source | Tool name | Key | Target |
|---|---|---|---|
| MCP server | `<server>_<tool>` (sanitised) | `mcp` | the tool name |
| OpenAPI import | one tool per enabled operation | `openapi` | the operation id |
| Composio | one tool per enabled action | `composio` | the toolkit |

All three default to `ask`.
