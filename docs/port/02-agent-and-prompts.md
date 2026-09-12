# 02 — Agent Loop and Prompt System

Port specification for the Rust/Tauri rewrite of Inertia's agent runtime. This
document describes the Electron/Node source exhaustively enough that it can be
reimplemented without reading the original. Every claim carries a
`file:line` reference into `D:\oss\inertia\src\main\` (read-only source of
truth; unchanged by this document).

Scope: the system prompt assembly (`agent/prompt.cjs`), the turn state
machine (`session/index.cjs`), tool execution and permission (`tools/tool.cjs`,
`permission/index.cjs`, `shared/permission.js`), hooks (`hooks/index.cjs`),
sub-agent orchestration (`team/crew.cjs`, `tools/builtin/{task,crew}.cjs`,
`shared/crew.js`), and the provider transports (`llm/anthropic.cjs`,
`llm/openai.cjs`, `llm/context.cjs`, `llm/cache.cjs`).

---

## 1. Prompt architecture

### 1.1 Where the prompt lives

The whole system prompt is one string, assembled by `build()` in
`src/main/agent/prompt.cjs:1019-1100`. It is sent as **one system message**
"because providers disagree about what a second system message means and one
of the disagreements is 'ignore it'" (`prompt.cjs:804-807`).

Two static text files are read once and cached in memory for the process
lifetime (`prompt.cjs:41-51`):

- `src/main/agent/prompts/core.txt` (150 lines) — the shared behavioral rules
  every agent gets, regardless of persona.
- `src/main/agent/prompts/subagent.txt` (19 lines) — appended instead of
  nothing extra when `mode === "subagent"`.

### 1.2 Section order (exact)

`build()` assembles a `sections` array and joins the non-null entries with
`"\n\n---\n\n"` (`prompt.cjs:1099`). In order:

1. **`identity(agent)`** — the agent's own persona. Heading: `# Who you are`.
   Opens the *whole document*, ahead of the shared core rules, deliberately:
   "an agent whose own configuration opens the document is an agent that
   reads everything below as addressed to the character it has already been
   told it is" (`prompt.cjs:1063-1067`). Renders:
   - `You are {name}, the {role.toLowerCase()} on this person's team.` (or
     just `You are {name}.` with no role) — `prompt.cjs:770-776`.
   - `agent.description`, verbatim, if present.
   - If `agent.systemPrompt` (or legacy `agent.prompt`) is set: a framing
     paragraph naming who wrote it and why it should be trusted, **then** the
     instruction verbatim. Exact framing text (`prompt.cjs:783-789`):
     > "What follows is the standing instruction this person wrote for you, in
     > this app's agent editor, before the conversation began. It is theirs and
     > it is deliberate. Read it as part of your instructions - it did not reach
     > you from a file, a page, a tool result or another agent - and let it
     > settle what you are for, what you know about, and how you work within
     > your own subject."

     This exists specifically because an unattributed instruction block at the
     end of a prompt reads as a prompt-injection attempt and models were
     observed declining to follow their own configured persona
     (`prompt.cjs:754-763`).
   - Returns `null` (section omitted) only when the agent has no name, no
     description, and no standing instruction (`prompt.cjs:794`).

2. **`core`** — contents of `core.txt`, always present.

3. **`subagent`** (conditional) — contents of `subagent.txt`, only when
   `mode === "subagent"`.

4. **`promptForMode(conversationMode)`** (conditional, omitted for
   subagents) — one of four fixed paragraphs from `src/shared/modes.js`,
   selected by `conversationMode` (`chat` | `plan` | `autonomous` | `group`,
   default `"chat"`). See §1.5.

5. **`environment({...env, approval})`** — always present. See §1.6.

6. **`layout(env.cwd)`** (async, conditional) — a shallow directory listing.
   See §1.7.

7. **`machine(computer)`** (conditional) — sandbox computer description, only
   when the agent has one. See §1.8.

8. **`teamwork()`** (conditional) — delegation instructions, only when
   `canDelegate` is true (i.e. this turn's tool list actually holds
   `spawn`/`collect`/etc.). Heading: `# Working with other agents`.

9. **`workbench({canBrowse, canReadTerminal})`** (conditional) — the browser
   pane / terminal-read instructions. Heading:
   `# The browser and the terminal beside this conversation`.

10. **`room(groupRoom)`** (conditional) — only in a group conversation.
    Heading: `# Who is in this conversation`. See §1.9.

11. **`product()`** (conditional, omitted when `canSetUp === false`) — "what
    Inertia is made of" (agents, routines, skills, memories, computers, MCP
    servers, permission rules) and how to use the `inertia_*` tools to change
    them rather than editing files by hand.

12. **`apps(appList)`** (conditional) — Composio-connected apps. Heading:
    `<connected-apps>` block, no `#` heading.

13. **`person(profile)`** (conditional) — who the user is. See §1.10.

14. **`instructions(env.cwd)`** (async, conditional) — AGENTS.md /
    CLAUDE.md / rules-folder content. See §1.11.

15. **`skills(skillList)`** (conditional) — advertised skill names +
    descriptions only, never bodies. `<available_skills>` block.

16. **`memories(memoryList, {enabled})`** (conditional) — advertised memory
    titles + one-line summaries, plus fixed instructions for *writing*
    memories. `<memories>` block.

Every section function returns `null` when it has nothing to say, and `null`
entries are filtered out before joining (`prompt.cjs:1099`, `.filter(Boolean)`
throughout). **A Rust port must preserve this null-omission behavior exactly**
— an empty section is not rendered as an empty heading, it is entirely absent,
because prompt space is deliberately not free (this reasoning recurs in nearly
every section's doc comment).

### 1.3 Environment block (verbatim structure)

`environment()`, `prompt.cjs:60-91`. Renders an `<env>` XML-ish block:

```
Here is the environment you are running in.

<env>
  Working directory: {cwd}
  Earlier turns in this conversation ran in: {previousCwd}        (only if cwd moved this conversation)
  The user changed it. What you said about the old folder was true then.  (only if cwd moved)
  Inertia workspace: {root}
  Platform: {platform}
  Shell: {shell}                                                   (only if shell given)
  Is a git repository: yes|no
  This folder is a git worktree of {worktree.root} on branch {worktree.branch}. Work here lands on that branch; the main checkout is untouched until it is merged. Call worktree_exit to go back.  (only if worktree)
  Today's date: {new Date().toDateString()}
  You are running on the model {model}.                            (only if model given)
  Approval: {promptForApproval(approval)}
</env>
```

The "folder moved mid-conversation" line exists to prevent a specific failure
mode: told the folder is now X after having said it was Y with nothing
recording the change, the model concludes it hallucinated the earlier answer
and starts second-guessing everything else it knows (`prompt.cjs:60-69`).
**Port note:** track `previousCwd` per conversation and diff it against
current `cwd` on every turn build.

`promptForApproval(approval)` (`shared/approval.js:107-124`) returns one of
three fixed sentences depending on the approval dial (`ask` | `edits` |
`auto`) — see §1.5 approval dial detail below.

### 1.4 Working-folder layout block

`layout(cwd, {limit=40})`, `prompt.cjs:110-145`. A **shallow** (non-recursive)
directory listing of `cwd`, skipping dotfiles and a fixed noise set:
`node_modules, .git, .next, dist, build, out, target, __pycache__, .venv,
venv, .cache, vendor, .turbo`. Directories sort before files, then
alphabetically. Capped at 40 entries with `...and N more`. Returns `null` if
the directory can't be read (permission error, etc. — silently omitted, no
error text, because "the tools will say so far more precisely the moment one
is used", `prompt.cjs:120-122`).

```
This is your working folder. It is the boundary of your file tools:
reading, searching or writing outside it asks the user first, so say what
you need rather than looking for a way around.

<working-folder>
  {cwd}
  {entries, dirs first with trailing /, space-joined}
  ...and {N} more
</working-folder>
```

### 1.5 Conversation modes (chat / plan / autonomous / group)

Defined in `src/shared/modes.js`. A mode is **exactly two things** and no
more (`modes.js:11-20`): a paragraph in the prompt, and a set of tool ids
withheld from the model's tool list. The withholding is what makes it a
guarantee rather than a request — "a model told 'do not build anything' while
holding `write` and `shell` will, three steps into a promising idea, build
something" (`modes.js:16-18`).

`MUTATING` tools (withheld in `chat`, present in `plan`/`autonomous`/`group`):
`write, edit, patch, file_copy, file_move, file_folder, file_delete, shell,
shell_kill, shell_logs, shell_list, shell_write, worktree_enter,
worktree_exit, task, later, spawn, collect, team, agent_send, computer_act,
computer_run, computer_write, computer_open, computer_launch, inertia_save,
inertia_remove, inertia_set_picture, inertia_set_rules, inertia_connect_app`
(`modes.js:32-68`).

`READ_ONLY` tools (present in every mode including `chat`): `read, lsp, ls,
glob, grep, skill, question, todowrite, inertia_list, inertia_get,
computer_observe, computer_read, computer_list, computer_page_text`
(`modes.js:71-86`).

`PRESENT_PLAN = "present_plan"` — only available in `plan` mode; calling it
is literally how Plan mode ends (see below).

`GROUP_ONLY = ["invite", "handover", "part"]` — withheld everywhere except
`group` mode.

`toolsForMode(tools, id)` (`modes.js:291-295`) filters a tool list by id
against the withhold-set — **withholding, not allow-listing**, so a
newly-added tool (an MCP server's, an imported API's) is visible in every
mode by default unless explicitly added to `MUTATING`/`GROUP_ONLY`.

Four modes, `DEFAULT_MODE = "chat"`:

- **`chat`** — `withhold = [...MUTATING, ...GROUP_ONLY, PRESENT_PLAN]`.
  Prompt heading `# This turn is a conversation`. Model can read/search but
  not act; told to offer switching to Plan mode for real work.
- **`plan`** — `withhold = [...MUTATING, ...GROUP_ONLY]` (keeps
  `present_plan`). Prompt heading `# This turn is for planning`. Must end by
  calling `present_plan`, which is the *only* way out of Plan mode — "the
  model cannot approve its own plan" (`modes.js:91-93`). Approving the plan
  (a user UI action, not a tool) switches the conversation to `autonomous`.
- **`autonomous`** — `withhold = [...GROUP_ONLY, PRESENT_PLAN]`. Prompt
  heading `# This turn is for building`. Full tools; told to finish without
  stopping to ask, verify what it builds, and stay inside the plan's scope.
- **`group`** — `withhold = [PRESENT_PLAN]`. Prompt heading
  `# You are in a group chat`. See §1.9 for the room mechanics this pairs
  with. This is the mode text that defines the `@handle` end-of-message
  addressing protocol, the `pass` single-word no-op reply, and the rule that
  the person's word always overrides the room's plan.

The picked mode's `.prompt` string is inserted directly after `core`/`subagent`
in the assembled document (`prompt.cjs:1073`), *not* wrapped in anything
else.

### 1.6 Approval dial (independent of mode)

`src/shared/approval.js`. **Orthogonal** to conversation mode: mode decides
which tools a turn *holds*; approval decides whether the tools it holds *stop
to ask*. Three positions, `DEFAULT_APPROVAL = "ask"`:

- `ask` — the permission ruleset as configured, unmodified.
- `edits` — file-edit calls (`EDIT_KEYS = ["edit"]`) go through without
  asking; everything else still asks.
- `auto` — anything a rule would have asked about is allowed, **except**
  `ALWAYS_ASK = ["delete_everything"]` which can never be waved through by
  any dial position, and except the "doom loop" guard (see §3.4) which
  becomes an outright **deny** under `auto` rather than a bypass — "a fourth
  identical call is a question... and with nobody to put it to, the honest
  answer is to stop the loop" (`approval.js:83-89`).

`resolveAsk(approval, key)` (`approval.js:91-96`) is the function that turns
a ruleset verdict of `ask` into a final `allow | ask | deny`, used by both
the prompt (`promptForApproval`) and the runtime permission check
(`permission/index.cjs:112-123`) — **these must produce the same answer**;
port them from one shared implementation.

`promptForApproval(approval)` produces one of three exact sentences embedded
in the `<env>` block (§1.3); see `approval.js:107-124` for exact text.

### 1.7 Machine / sandbox computer block

`machine(computer)`, `prompt.cjs:546-661`. `null` unless the agent has a
sandbox computer assigned. Two variants depending on `computer.hasDesktop`:

- **Headless** (`hasDesktop === false`): states this is "a plain base image:
  shell and files only, no desktop and no browser."
- **Full desktop**: states OS (Debian 12, user `agent` with sudo), installed
  tooling (`node 22, python3, git, ripgrep, jq, curl, build-essential`), an X
  display at `:99, 1600x900` running Chromium, and appends a long block of
  computer-use instructions (coordinate reading off on-screen rulers,
  look/act/check loop, one-`computer_act`-call-does-a-sequence guidance,
  CAPTCHA/2FA handoff to the human, etc). This entire block is reproduced
  verbatim at `prompt.cjs:603-657` — copy it exactly if porting a computer-use
  feature; it is highly failure-mode-driven prose, not filler.

Crucially states the **two-machine boundary**: `computer_*` tools reach the
sandbox; `read/write/edit/ls/shell` reach the machine Inertia itself runs on;
nothing is shared between them (`prompt.cjs:575-581`). If both a sandbox and
a local browser pane exist, a further paragraph (`prompt.cjs:592-601`)
disambiguates `browser_*` (the pane in the user's own window, sees their
localhost/cookies) from `computer_*` (the sandbox's own browser).

### 1.8 Team / delegation and workbench blocks

`teamwork()` (`prompt.cjs:890-953`) — see §4 for the semantics it documents;
only included when `canDelegate` is true for this turn.

`workbench({canBrowse, canReadTerminal})` (`prompt.cjs:823-874`) — describes
`browser_navigate/read_page/click/type/console/network/evaluate/screenshot`
(only if `canBrowse`) and `terminal_read` (only if `canReadTerminal`). Core
instruction: "Do not report a UI change as done on the strength of the code
alone. Open it, read the console, and say what you saw" (`prompt.cjs:856-857`)
— this maps directly onto the "user verifies UI themselves" policy noted for
this Rust port: **the Tauri port's agent must never auto-drive a browser to
self-verify unless the user has explicitly wired that capability in**, mirroring
Inertia's own `canBrowse` gate.

### 1.9 Group room block

`room({roster, me, permissions, custom, person})`, `prompt.cjs:968-1017`.
Only present in a multi-agent group conversation. Structure:

```
# Who is in this conversation

You are **{self.name}**, `@{slugOf(self.name)}` to the others.
The person's messages arrive as "{person}:". Theirs is the final word.
Besides you and the person:                          (or "Only you and the person, for now. You can bring others in." if solo)
- **{name}** (`@{slug}`) - {role}. {description}
...

You may bring another agent in (`invite`, or name them with `@`), hand the conversation over (`handover`), leave when your part is done (`part`).   (or: "You cannot change who is in this conversation; the person decides that.")
At most {maxAgents} agents can be in here at once.

End your message on the `@handle` of whoever should answer it, and they
speak next. End without one and the floor goes back to the person, so a
question you leave unaddressed is a question nobody has been asked. The
person can also name any of you with `@`, and that is who answers next -
whatever the room had planned.

## How this person wants their agents to work together

{custom, verbatim}
```

The speaker's own entry is deliberately excluded from the roster list but
**included** with its own handle line at the top — "every agent [gets] its
handle, the speaker's included, so replies can name each other"
(`prompt.test.js:314-320`). `slugOf` comes from `src/shared/group.js`.

The turn-taking mechanism is **entirely prompt-driven**: there is no
`next_speaker` tool call. An agent's reply ending in `@handle` is what hands
the floor to that agent; a reply naming nobody returns the floor to the
person. This is the *entire* mechanism (`modes.js` group prompt: "No tool,
no ceremony - the handle is the whole mechanism").

### 1.10 Person / user profile block

`person(profile)`, `prompt.cjs:481-529`. Returns `null` if the profile has
nothing (no name, bio, email, timezone, locale). Structure:

```
You work for {name} ({handle}). Call them {shortName}.    [shortName line omitted if it equals name]
Their email address is {email}. Do not use it anywhere they did not ask you to.
They are in the {timezone} time zone. Write times in it unless asked otherwise.
They read dates and numbers in the {locale} format.

They describe themselves and how they want to be worked with:

{bio, verbatim}
```

Only `preferences.timezone`/`.locale` that are **not** the literal string
`"system"` are surfaced — `"system"` means "whatever the OS says", which is
already covered by the environment block's own date/platform info, so
repeating it risks the two disagreeing (`prompt.cjs:487-491`). Avatar,
avatar initials, and any purely-visual preference are deliberately excluded —
"neither is something an agent can act on" (`prompt.cjs:517-518`).

### 1.11 Project instructions (AGENTS.md / CLAUDE.md / rules)

`instructions(cwd)` → `projectInstructions(cwd)`, `prompt.cjs:176-323`. This
is Codex's/Claude Code's scheme, explicitly copied intentionally
(`prompt.cjs:160-167`):

1. **Find the project root**: nearest ancestor of `cwd` containing a `.git`
   directory, walking up at most `MAX_ASCENT = 24` levels; falls back to
   `cwd` itself if none found (`projectRoot`, `prompt.cjs:207-216`).
2. **Walk root → cwd inclusive** (`chain()`, `prompt.cjs:233-245`), and in
   **each** folder along that chain:
   - Read the first file that exists, in priority order:
     `AGENTS.override.md` > `AGENTS.md` > `CLAUDE.md` > `CONTEXT.md`
     (`INSTRUCTION_NAMES`, `prompt.cjs:181`).
   - Additionally read `CLAUDE.local.md` if present, **beside** whichever of
     the above was chosen, never instead of it (`prompt.cjs:184, 262-263`).
3. Additionally read every `.md` file (in filename order) inside
   `.inertia/rules/` and `.claude/rules/`, at both the project root and the
   working folder (deduplicated if they're the same folder)
   (`RULES_DIRS`, `prompt.cjs:186-187, 266-282`).
4. Files are concatenated **furthest (root) first, nearest (cwd) last**, so
   the nearer file is "read last and weighed most" — this is the resolution
   order when two files disagree (`prompt.cjs:162-163, 293`, confirmed by
   test at `instructions.test.js:49-56`).
5. HTML comments (`<!-- ... -->`) are stripped before counting bytes or
   sending — "for the maintainer" not the model (`prompt.cjs:219-221`).
6. Empty files (after stripping) do not count as "the folder's file"; the
   next name in priority is tried (test at `instructions.test.js:93-98`).
7. **Total budget: `MAX_INSTRUCTION_BYTES = 32 * 1024`** across every file
   combined (`prompt.cjs:178`). When the budget runs out mid-file, that
   file's body is truncated with a `[... cut here: the instructions exceed
   32 KB]` marker; if a whole subsequent file can't fit even its own header,
   it and everything after it is dropped, and a note is appended: "Not every
   file fitted; the rest was left out. Read the files themselves if you need
   what was cut." (`prompt.cjs:292-320`).

Rendered structure:

```
The project at {root} includes the following instructions, from more than one file.
Follow them. They were written by the user and they outrank your general habits.
A file applies to the folder it sits in and everything under it. They are given furthest first; when two disagree, the nearer one to the folder you are working in wins.

## From {relative path to file}

{body}

## From {relative path to next file}

{body}
```

### 1.12 Skills and memories (advertise-only, load-on-demand)

Both follow the identical "title + one line, never the body" bargain
(`prompt.cjs:326-355, 395-407`), for the same reason: paid on every turn
whether used or not.

**Skills** (`skills(list)`, `prompt.cjs:357-393`):
```
<available_skills>
  <skill>
    <name>{name, escaped}</name>
    <description>{summarised description, escaped, max 300 chars}</description>
  </skill>
  ...
</available_skills>
```
A skill with no description is still listed (not hidden) with a fixed
placeholder: "No description was written for this skill, so load it only if
the user names it." A duplicate `name` across two skills falls back to
rendering both by their (unique) folder `id` instead — resolving a name to
"whichever it finds first" is worse than an unfamiliar-looking name
(`prompt.cjs:361-374`).

**Memories** (`memories(list, {enabled})`, `prompt.cjs:408-464`): if
`enabled` is false, section is `null` entirely. If enabled but the list is
empty, a shorter block just states "You have no memories yet..." plus the
saving-rules paragraph. If non-empty:
```
<memories>
  <memory>
    <title>{title, escaped}</title>
    <summary>{one-line summary, escaped}</summary>
    <pinned>true</pinned>          (only if pinned)
  </memory>
  ...
</memories>
```
followed unconditionally by the fixed "saving rules" paragraph instructing
the model to call `memory_save` proactively, one fact per memory, with
`replaces` on corrections, tagging scope `project` vs `global`
(`prompt.cjs:415-424`).

Selecting *which* memories make it into the budget-capped list is done in
`src/shared/memory.js: forPrompt()` (not in `prompt.cjs` itself) — see §1.13.

### 1.13 Memory selection algorithm (feeds `memories()`)

`src/shared/memory.js`. Pure, dependency-free (shared between main process
scoring and renderer display). Key exported knobs:

- `INJECT_BUDGET_BYTES = 2560` — total byte budget for the advertised list
  per turn (≈ 20 entries at title+line each).
- `INJECT_MAX = 24` — hard cap on entry count regardless of byte budget.
- `MAX_SUMMARY = 160` — chars per rendered summary line.
- BM25-style relevance scoring (`K1=1.2, B=0.75`) over a simple
  lowercase/alnum tokenizer with a short English stopword list
  (`memory.js:166-282`).
- `forPrompt(memories, {query, budget, max, now})` (`memory.js:307-363`) —
  **three-tier ranking**: tier 0 = pinned or `kind === "handover"` (always
  included regardless of budget for pinned ones); tier 1 = relevant to the
  current query (BM25 > 0); tier 2 = "has proved useful before" (sorted by
  `useCount` then recency). Cut to `INJECT_BUDGET_BYTES`/`INJECT_MAX`,
  never dropping a pinned entry for length.
- `applicable(memories, folder)` filters project-scoped memories to the
  current working folder (comparing normalized paths case-insensitively,
  `sameFolder()`), always keeping `scope: "global"` ones.
- `approved(memories)` excludes memories still pending human review.
- Secret-shape detection (`looksSecret`, `SECRET_SHAPES`,
  `memory.js:472-488`) blocks storage of anything matching API-key /
  token / private-key patterns — `isStorable()` is checked **before**
  writing, never after.

### 1.14 Provider-specific prompt variants

**There is no per-provider prompt text variant.** The exact same assembled
string from `build()` is sent to every provider. What differs is *transport
packaging*, not content:

- **Anthropic** (`llm/anthropic-messages.cjs: toAnthropicMessages`): the
  system string is placed in the Messages API's dedicated top-level `system`
  field, as a **one-element array** `[{type: "text", text: systemText}]` —
  array form specifically because that is "where a cache breakpoint has to
  be hung" (`anthropic-messages.cjs:218-224, 297-300`). Sending it as a
  `role: "system"` message is rejected outright by that API.
- **OpenAI-compatible** (`llm/openai-messages.cjs: toRequestMessages`): the
  same string becomes the first message in the array, `{role: "system",
  content: system}` (`openai-messages.cjs:82-83`).

No other content difference exists. A Rust port's prompt-builder should be
provider-agnostic and produce one string; the transport layer decides how to
frame it.

### 1.15 Prompt caching (Anthropic only) — composition seams

`src/main/llm/cache.cjs`. This is the concrete answer to "what is static vs
dynamic per turn, and where do cache breakpoints sit":

Mechanism: Anthropic's `cache_control: {type: "ephemeral", ttl?}` marker on
the **last block** of a list means "everything up to and including this
block is one cacheable prefix." At most **4** markers per request, and their
order is fixed by request shape: tools, then system, then messages.

Where Inertia places them (`withCacheControl`, `cache.cjs:114-161`):

1. **Last tool schema** — tool schemas are static within a session and the
   single largest fixed cost (thousands of tokens for ~20 tools).
2. **Last system block** — the assembled prompt from §1.1-1.13. Static
   *within a turn* but can change turn-to-turn (memories recalled, cwd
   layout, date). It still caches because within one multi-step turn the
   system prompt is resent unchanged at every step.
3. & 4. **Last two `user`-role messages** — in this wire protocol a *tool
   result* is a user message, so every step of the agent loop ends with one.
   Marking here means the breakpoint written at step N is read back in full
   at step N+1; only the newly-appended tokens are paid for fresh. Two
   (not one) breakpoints are used because the newest write may not have
   "landed" server-side by the time the next request fires — the older
   breakpoint is the fallback (`cache.cjs:28-34`).

A marker is only placed if the prefix at that point is
`>= MIN_PREFIX_TOKENS = 2048` (the larger of Anthropic's own two floors,
conservatively chosen) — otherwise the marker would be silently ignored by
the API while still consuming one of the 4 slots (`cache.cjs:56-68`).

TTL: `1h` (`extended-cache-ttl-2025-04-11` beta header required) by default,
falling back to `5m` if the endpoint rejects the beta (`TTL_HOUR`,
`TTL_FIVE_MINUTES`, `EXTENDED_TTL_BETA`, `cache.cjs:48-53`). The choice of an
hour over Anthropic's 5-minute default is deliberate for a desktop app: "a
person reads the answer, thinks, and types a follow-up, and five minutes is
exactly long enough to lose that race" (`cache.cjs:36-43`).

**Composition-seam summary for the port:**

| Layer | Static within | Changes | Cache-relevant |
|---|---|---|---|
| `core.txt` / `subagent.txt` | process lifetime | app version | yes (part of system prefix) |
| mode paragraph | per-mode | user switches mode | yes |
| environment block | per-turn | cwd, date, approval dial | yes but small, still part of same system-block breakpoint |
| working-folder layout | per-turn (re-listed each build) | fs changes | yes |
| project instructions | until files change on disk | rare | yes |
| skills/memories advertised list | per-turn (memory selection is query-dependent) | every turn (query changes) | yes, but this is the **last** thing appended to system, so it's "free" to change — the cache breakpoint sits *after* it |
| tool schemas | per-turn (mode/tool-access gating) | rarely within a turn | yes, separate breakpoint |
| conversation history | grows every step | every step | breakpoints 3 & 4, sliding forward |

Practical implication for a Rust port: **build the system prompt as a single
string per request** (cheap — it is not literally memoized across turns in
the source, `build()` runs fresh every turn), but preserve the Anthropic
transport's separate `cache_control` marker placement logic verbatim, since
that is the actual performance/cost lever, not memoizing the JS string
itself.

---

## 2. Prompt composition seams — summary

Restating §1 as a strict list of "what varies, and at what granularity":

- **Fixed at build time (compiled into the binary/app bundle):** `core.txt`,
  `subagent.txt`, all four mode paragraphs, all the fixed instructional
  prose in `teamwork()`/`workbench()`/`product()`/`machine()`/memory-saving
  rules. These never change per-user; a Rust port can `include_str!` them.
- **Fixed per agent record (changes when the user edits the agent in
  settings):** `identity()` output (name, role, description, systemPrompt).
- **Fixed per workspace, slow-changing:** `instructions()` (AGENTS.md etc,
  re-read from disk every turn but rarely edited), `skills()` list.
- **Per-turn, cheap to recompute:** `environment()` (date always changes!),
  `layout()` (fs snapshot), `person()` (profile, rarely changes but cheap),
  `apps()` (connected-app list).
- **Per-turn, semantically dynamic:** `memories()` — the *selection* changes
  based on the incoming user message (BM25 query relevance), even though the
  underlying memory store doesn't change turn to turn.
- **Per-turn, structural (changes which tools exist, hence caching cohort):**
  `canDelegate`, `canBrowse`, `canReadTerminal`, `canSetUp`, `mode`,
  `conversationMode`, `approval`, `groupRoom` — all passed explicitly into
  `build()` rather than being inferred, "because the tool list is the only
  honest answer" (`prompt.cjs:1032-1042`).

---

## 3. The agent turn loop (state machine)

Implementation: `src/main/session/index.cjs` (1996 lines). Entry point
`run(options)` → `converse(options, steering)` (`index.cjs:486-508`). There
is no separate "anthropic-loop.cjs" — the loop is provider-agnostic; provider
selection happens via `src/main/llm/client.cjs` and the transports in
`llm/anthropic.cjs` / `llm/openai.cjs` implement an identical `onEvent`
contract so the loop cannot tell which it is talking to.

### 3.1 Setup phase (once per turn, before the loop)

1. Register the turn in a `running` map keyed by `turnId` so `steer(id,
   text)` can reach it later (`index.cjs:488`).
2. Resolve the permission ruleset as **three merged layers**: defaults →
   workspace → agent's own (`rulesFor`, `index.cjs:456-471`), consistent with
   `shared/permission.js: merge()`'s "later wins" semantics.
3. Build the mode-filtered tool list (`toolsForMode`), apply on-demand tool
   gating (only `ALWAYS_LOADED` tools — see `shared/tool-access.js:63-95` —
   are present up front if `tool_access === "on-demand"`; the rest wait
   behind the `load_tools` tool).
4. Recall relevant memories, build the system prompt via `prompt.build(...)`.
5. Load hooks (`hooks.load({root, cwd, agent})`, see §3.6) and fire
   `SessionStart`/`SubagentStart`, then `UserPromptSubmit`. **If
   `UserPromptSubmit` blocks, the turn terminates immediately** with
   `stopped: "blocked"` and an `error` event — no LLM call is ever made
   (`index.cjs:1059-1083`).
6. Attempt to recall a cached compaction summary if its fingerprint still
   matches the incoming transcript prefix (`compactions.recall`,
   `index.cjs:777-784`; see §3.7).

### 3.2 Main loop

`while (steps <= MAX_STEPS)`, `MAX_STEPS = 100` (`index.cjs:94, 1091`). Each
iteration:

1. **Pause gate**: `await crew.waitWhilePaused(runId)` (only meaningful for
   spawned sub-runs, see §4), then check `signal.aborted` — the *only* place
   in the loop where a pause/steer/abort is safe to apply structurally
   (`index.cjs:1105-1106`).
2. Increment `steps`; re-check abort (`index.cjs:1108-1109`).
3. **Steering fold-in**: any text the user typed mid-turn
   (`steering.take()`) is appended to the transcript as user messages here —
   **never mid-batch**, only at the top of an iteration (`index.cjs:1119`).
4. Settled sub-agent runs (spawned via `spawn`) are announced into the
   transcript as a synthetic user note (`index.cjs:1135-1148`) — this is the
   "note from the system, not from the person" mechanism from
   `shared/crew.js: settledNotice()`.
5. A one-time context-usage warning note is injected if the previous step's
   request crossed `CONTEXT_WARN_AT = 0.85` of the window
   (`index.cjs:1150-1153`).
6. `lastStep = steps > MAX_STEPS`: the over-budget final iteration is sent
   with **`tools: []`** and an appended note forcing a prose-only wrap-up
   reply instead of another tool call (`index.cjs:1164, 1209-1210`).
7. **Pre-request compaction check** runs before *every* request, not only
   after a provider rejection (`index.cjs:1179-1198`; algorithm in §3.7).
8. **Provider call**: `client.streamChat(provider, apiKey, request, onEvent,
   {signal})`. The streaming callback demultiplexes transport events
   (`delta | reasoning | thinking | tool | notice | retry | error | done`,
   see §5) into local accumulator state and forwards to the loop's own
   `onEvent`.
9. Post-stream abort re-check (`index.cjs:1285`) — a reply cut off in flight
   is **not** appended to the transcript.
10. **Error branch** (`index.cjs:1287-1366`), tried in order:
    - context-overflow recovery: `isContextOverflow()` (llm/overflow.cjs) →
      force-compact and retry the *same* step (`steps -= 1`) — bounded to
      once per turn via a `recovered` flag (`index.cjs:1015, 1293-1312`).
    - image-refusal recovery: strip images from the request and retry, once
      per turn (`blind` flag, `index.cjs:736, 1317-1322`).
    - built-in-tool-refusal nudge: bounded by `MAX_BUILTIN_NUDGES = 2`
      (`index.cjs:177, 1327-1354`).
    - otherwise: **terminate**, `stopped: "error"`, fire `StopFailure` hook,
      emit an `error` event (`index.cjs:1355-1365`).
11. On success: push the assistant's transcript entry (text + tool calls +
    signed thinking blocks) (`index.cjs:1369-1378`).
12. **No tool calls requested** → completion branch:
    - if steering has pending text, `continue` rather than finishing
      (`index.cjs:1386`);
    - if sub-agent runs are still active under this turn, nudge the model
      once (bounded, `teamNudges < 1`) and `continue`
      (`index.cjs:1391-1423`);
    - fire `Stop`/`SubagentStop` hook: `stop` → terminate `stopped:
      "complete"`; `block` (bounded `STOP_HOOK_LIMIT = 5`) → append a note
      and `continue` (`index.cjs:1434-1468`);
    - otherwise **terminate**, `stopped: "complete"`, emit `done`
      (`index.cjs:1470-1473`).
13. **Tool calls requested** → emit a `step` event, group calls into
    parallel-safe batches, execute via `performCall` (§3.3-3.5). A batch
    outcome can set `stop`/`halted`/`refused`, leading to an early
    `return` with `stopped: "hook"` or `stopped: "refused"`, or falling
    through to loop again.
14. Re-enter loop; `steps <= MAX_STEPS` re-checked.

### 3.3 Tool call batching

Calls within one model turn that are independent are executed concurrently
("batches"); calls with dependencies (rare — the model itself decides by
issuing them in the same response) run through the same `performCall` path
per call. The exact batching predicate lives at `index.cjs:1476` onward —
port note: treat "all tool_use blocks in one assistant message" as the unit
that may run in parallel, matching the provider's own concurrency contract
(Anthropic/OpenAI both allow multiple `tool_use`/`tool_calls` per assistant
turn and expect all of them answered before the next request).

### 3.4 Per-call sequence (`performCall`, `index.cjs:1487-1851`)

1. **Doom-loop guard**: fingerprint = `tool.id + JSON(args)`. If the last
   `LOOP_THRESHOLD = 3` calls are byte-identical, a `permission.ask({key:
   "doom_loop", ...})` is forced before allowing a 4th identical call through
   (`index.cjs:1536-1572`). Under the `auto` approval dial this **denies**
   rather than silently passing (see §1.6).
2. **`PreToolUse` hook** fires (`index.cjs:1587-1608`) — may block outright
   (tool never executes), rewrite the arguments (`updatedInput`), or preempt
   the permission ask with `decision: allow|ask|deny`.
3. **Tool execution** via `runTool(tool, parsedArgs, ctx)`
   (`src/main/tools/tool.cjs:109-192`) — see §3.5 for the tool contract in
   full.
4. Result folded back into the transcript as a `role: "tool"` entry keyed by
   `toolCallId`.
5. `PostToolUse` hook fires with the result; may again block or append
   context.

A `PermissionDenied` thrown from `runTool` surfaces as `result.ok === false,
metadata.error === "denied"`. If it specifically carries
`metadata.refused === true` (the **user** explicitly clicked "no", as
opposed to a static rule denying it), the **entire batch is stopped** and the
turn terminates with `stopped: "refused"` — "one 'no' cancels the turn... when
a model fires three tool calls and the user refuses the first, the other two
are refused too" (`permission/index.cjs:20-22`, `index.cjs:1817-1819,
1889-1897`).

### 3.5 The tool contract (`tools/tool.cjs`, `tools/CONTRACT.md`)

Every tool — built-in, MCP-sourced, OpenAPI-imported, Composio — is the same
shape, so permission/history/truncation/cancellation are written once:

```js
defineTool({
  id: "read",                        // unique, lower-case, what the model calls
  description: "...",                // sent verbatim to the model
  parameters: JSONSchema,             // flat only: no $ref, no anyOf, no additionalProperties:true
  permission: {
    key: "read",                     // settings-screen rule key (coarser than tool id — see shared/tools.js)
    target: (args) => args.filePath, // what a rule is WRITTEN ABOUT
    always: (args) => "*",           // what "always allow" remembers (can differ from target)
  },
  render: (args) => args.filePath,   // one-line UI label while running
  normalize: (rawArgs, ctx) => rawArgs, // optional pre-schema-check repair, sees ctx
  async execute(args, ctx) { return { title, output, metadata, images? }; },
});
```

`runTool(tool, rawArgs, ctx)` (`tool.cjs:109-192`) — the fixed pipeline every
call goes through:

1. `tool.normalize?.(rawArgs, ctx)` — best-effort repair of a shape the
   schema would otherwise reject; errors here are swallowed and the raw args
   fall through to the schema check, which reports the problem in
   model-actionable words.
2. `validate(tool.parameters, raw)` (`schema.cjs`) — coerces types (declared
   `integer` really becomes a number), checks required fields. Failure
   returns `{ok:false, output:"Invalid arguments for {id}: {error}"}` **as a
   tool result**, not a thrown exception — the model reads it and retries.
3. **Permission ask**, computed from the tool's own `permission.target`/
   `.always` functions, then handed to `ctx.ask(...)` (bound to
   `permission.ask` in the session, see §3.6). Throws `PermissionDenied` if
   refused; this propagates up through `execute()`'s try/catch.
4. `tool.execute(args, ctx)` runs.
5. Output is passed through `truncate.output()` — auto-truncated unless the
   tool already set `metadata.truncated` itself.
6. Return shape always includes `ok`, `title`, `output` (text — "a tool
   result is a message and a message is text"), optional `images` (data URLs,
   turned into a *following user message* by the session, since no
   text-only-tool-result-shape provider accepts an image inside a `role:
   "tool"` message), `metadata` (UI-only, model never sees it), `durationMs`.
7. **Failure handling**: `ToolError` (recoverable — bad path, bad pattern,
   non-zero exit) is caught and returned as `{ok:false, output:
   error.message}` — the model sees the error text and can retry a different
   way. Only cancellation (`ctx.signal.aborted`) re-throws past this
   boundary. `PermissionDenied extends ToolError` with `retryable: false`
   and (when it was a human "no") `refusedByUser: true`.

`ctx` fields available to `execute` (`CONTRACT.md`): `cwd`, `root`
(workspace folder), `signal` (AbortSignal), `sessionId`/`messageId`/`callId`,
`agent` (name), `ask(request)` (permission — throws `PermissionDenied`),
`update({title, metadata})` (stream partial state mid-run, used heavily by
`spawn`/`task` to report sub-agent progress), `tools` (the other tools this
turn holds, for composing tools like `task`).

**Port implication:** in Rust, model this as a `trait Tool` with associated
`execute(args, &mut Ctx) -> Result<ToolOutcome, ToolError>`, a JSON-Schema
`parameters()`, and a `permission()` descriptor struct — the permission
target/always closures map naturally to two `fn(&Args) -> Option<String>`
methods or an enum. Keep `normalize` optional. Truncation and permission-ask
should be middleware wrapping every tool call uniformly, exactly as
`runTool` does, so adding a new tool source (MCP, etc.) never has to
re-implement them.

### 3.6 Permission-check flow (detail)

`ctx.ask` is bound per-turn to (`index.cjs:1667-1706`):
```
permission.ask({
  ...request, sessionId, rules, signal, agent: agent?.name,
  unattended, approval, preapproved, forceAsk, beforeAsk,
})
```

`permission.ask(request)` (`src/main/permission/index.cjs:82-206`) resolution
order — **this exact order must be preserved**, it is the security-relevant
core:

1. Merge session "always"-granted rules on top of the passed-in ruleset
   (`permission.merge`, later wins) and `evaluate(rules, key, target)`
   (`shared/permission.js:239-268` — glob match on both tool-id pattern and
   target pattern, most-specific match wins where tool-pattern specificity
   dominates target-pattern specificity: `score = specificity(tool)*100000 +
   specificity(pattern)`).
2. `deny` → **throw immediately**, quoting the matching rule in the message
   text so the model can explain it to the user rather than retrying
   (`permission/index.cjs:88-97`).
3. `allow` (and hook didn't force a re-ask) → resolve immediately, no
   prompt.
4. A `PreToolUse` hook that already said `decision: allow`
   (`preapproved`) → resolve immediately without a prompt (spares the user a
   card for something their own script already vetted).
5. Approval dial (`resolveAsk(approval, key)`, §1.6): `allow` → resolve
   without prompting (unless a hook set `forceAsk`); `deny` → throw (this is
   the *only* place the doom-loop guard becomes a hard stop, under `auto`).
6. A `PermissionRequest` hook (`beforeAsk`) gets first refusal — may itself
   answer `allow`/`deny` before a human sees anything. Runs **before** the
   unattended check, deliberately, so a routine with a permission hook need
   not blanket-refuse everything.
7. `unattended` (a routine's turn, no window watching) → **always** throw
   `PermissionDenied`, telling the model to finish the rest of the work and
   report what it wanted to do instead of looping.
8. No renderer listening at all (`!notify`) → throw (headless safety net).
9. Turn already aborted → throw immediately rather than opening a UI card
   for a dead turn.
10. Otherwise: create a pending question `{id, sessionId, agent, key,
    target, always, title, metadata, at}`, call `notify({type:"asked",
    question})` (this is the IPC event the renderer surfaces as a UI
    prompt), and return an unsettled `Promise`. An `abort` on `signal`
    rejects this promise with `PermissionDenied("The user stopped the
    turn.")` and emits `notify({type:"settled", id})`.

`reply(id, answer, message)` (`permission/index.cjs:216-251`) — the user's
answer, three shapes:

- `"reject"` → constructs a `PermissionDenied` (optionally carrying the
  user's own rejection message, fed back to the model as redirection — "no,
  use the staging bucket" is worth more than a bare refusal), marks
  `refusedByUser: true`, and **also calls `rejectSession(sessionId, ...)`**
  which rejects every *other* still-pending question in the same session —
  this is the "one no ends the whole batch" behavior.
- `"always"` → appends `permission.rule(key, "allow", question.always)` to
  the session's in-memory grant table (`granted: Map<sessionId, rules[]>` —
  **session-scoped, in-memory only**, never silently written to the
  workspace's persisted ruleset; a deliberate choice so a mid-conversation
  grant doesn't become a permanent setting without the user visiting
  Settings). Then re-evaluates every other pending question in the session
  against the updated rules (`settleCovered`) so anything the new grant now
  covers resolves without a separate prompt.
- anything else (`"once"`) → resolves just this one call.

Grants live in `granted: Map<sessionId, Rule[]>`, dropped on
`permission.forget(sessionId)` (new conversation) — **not** persisted across
app restarts, and **not** shared across sessions (a sub-agent's `sessionId`
is `parent/task-N`; questions from it surface to the same person because
`rootSession()` strips to the first path segment, but its grants are its own
map entry, separately scoped).

Ruleset merge order (defaults → workspace → agent), rule matching (glob
`*`/`?` only, specificity-ranked), and the flat/nested config encodings all
live in `src/shared/permission.js` — this file is explicitly
"deliberately pure and dependency-free" so both renderer (editing rules) and
main process (enforcing them) share one implementation
(`permission.js:14-18`). **Port this as one crate/module used by both the
UI and the enforcement path**, not duplicated.

### 3.7 Compaction (context management)

Two layers.

**A. Per-request compaction** (`src/main/llm/context.cjs`), run before every
request (`index.cjs:1179-1198`):

- Token estimate: 4 characters per token (`estimateTokens`), a deliberately
  conservative heuristic, not a real tokenizer.
- Context window resolved from (in order): the provider's own reported model
  list entry → a shipped table (`shared/models.js`, needed because
  Anthropic's endpoint reports no window at all) → `DEFAULT_CONTEXT_TOKENS =
  32768` fallback (biased low on purpose: guessing high risks a hard
  rejection, guessing low only costs one extra compaction call).
- **Trigger**: only runs if `requestTokens > floor(window * COMPACT_AT)`,
  `COMPACT_AT = 0.7`.
- **Step 1 (prune, free)**: `pruneOldToolOutput` — any tool-result entry
  older than the "keep" window (`KEEP_AT = 0.3` of context window, counted
  back from the end) and longer than `PRUNE_MIN_CHARS = 600` is truncated to
  its first `PRUNE_KEEP_CHARS = 200` chars with a note appended: "Call the
  tool again if you need them." If this alone gets the request under the 0.7
  line, compaction stops here (no model call spent).
- **Step 2 (summarize, costs one LLM call)**: if still too big, `cutPoint()`
  finds the oldest safe cut boundary — walking backward it **never** orphans
  a `tool`-role entry from the assistant message that requested it (a cut
  between a tool call and its result is a guaranteed 400 on the next
  request). Everything before the cut is digested to plain text
  (`digest()`) and summarized by one no-tools LLM call using a fixed
  instruction with **mandatory headings**: `## Goal`, `## Constraints and
  preferences`, `## Done`, `## Decisions`, `## Open`, `## Worth keeping`
  (`SUMMARY_INSTRUCTION`, `context.cjs:203-229` — write "none" rather than
  omit a heading, so nothing important silently vanishes for lack of a
  place to put it). Only proceeds if `>= MIN_DROPPED = 4` messages would be
  dropped (else not worth a round trip), except when `force: true` (a
  provider already rejected the request as too long) where the floor drops
  to 1 message and the "keep" share is **halved** (`share = KEEP_AT / 2`).
  A failed summarizer call falls back to a truncated raw digest
  (`fallbackSummary`) rather than losing the turn.
- The dropped prefix is replaced by exactly **one** pinned `summaryEntry()`
  message (`shared/summary.js` — shared with the composer's manual `/compact`
  command so the two never produce different shapes).
- A `warning`-type event is emitted describing what happened, every time
  compaction actually runs.

**B. Cross-turn compaction cache** (`src/main/session/compactions.cjs`),
top-level turns only (`depth === 0`, i.e. not for sub-agent runs):

- At turn start: `compactions.recall(threadId, incomingHistory)` checks
  whether a previously-stored summary's fingerprint (SHA1 over the JSON of
  the exact prefix it replaced) still matches the current transcript's
  prefix. If it matches, the transcript is rebuilt as `[summaryEntry(stored
  summary), ...incoming.slice(coveredCount)]` with **zero model calls** —
  this is the mechanism that makes reopening a long thread cheap.
- Any time compaction actually runs during the turn, the new summary and its
  coverage count are persisted to `cache/<sha1(threadId)>.json`
  (`compactions.remember`).
- A mismatch (user edited/deleted an earlier message) invalidates the cache
  entry; cost is exactly one re-summarization, described in the source as
  the only failure mode by design.
- `sweep({days: RETENTION_DAYS = 60})` prunes stale cache files; unrelated to
  the turn loop itself.

**Forced compaction on provider rejection**: `isContextOverflow(message,
status)` (`llm/overflow.cjs`) classifies a 400/413 provider error as "too
long" via a phrase list (`OVERFLOW`) with an explicit exclusion list
(`NOT_OVERFLOW`: rate-limit/quota/billing/throttle/overloaded/concurrent
wording) checked **first** and winning — misclassifying a rate limit as
overflow would discard half the conversation to solve a problem a 2-second
wait would fix. A bodyless 413, or a bodyless 400, is also treated as
overflow (some gateways answer that way). On a positive match, the loop
force-compacts and retries the **same** step (`steps -= 1`, so it doesn't
consume the step budget) — bounded to once per turn.

### 3.8 Hooks (lifecycle extension points woven into the loop)

`src/main/hooks/index.cjs`. Shape is Claude Code's, deliberately, "for the
scripts people have already written against it" (`hooks/index.cjs:12-16`).

Fixed event list, in the order they occur in a turn (`EVENTS`,
`hooks.cjs:44-57`): `SessionStart, UserPromptSubmit, PreToolUse,
PermissionRequest, PostToolUse, Notification, PreCompact, PostCompact,
SubagentStart, SubagentStop, Stop, StopFailure`.

Only `BLOCKABLE = {UserPromptSubmit, PreToolUse, PostToolUse, Stop,
SubagentStop}` may actually veto their event; a `block` decision on a
non-blockable event (e.g. `Notification`) is downgraded to a mere message,
never a veto (`hooks.cjs:575-580`).

Two handler kinds, both normalized to the same outcome shape:
- `command` — a subprocess, given the event as JSON on stdin, timeout-bound
  (default 60s, `DEFAULT_COMMAND_TIMEOUT_S`, max 600s). Exit code 2 is the
  blocking exit (stderr becomes the reason). JSON on stdout is parsed for a
  richer contract (`continue: false` to stop the whole turn,
  `decision: "block"|"allow"|"deny"|"ask"`, `hookSpecificOutput.
  updatedInput` to rewrite tool arguments, `additionalContext` to inject text,
  `systemMessage` shown to the person).
- `prompt` — a judgment call put to the **same model already running the
  turn**, no tools, answer constrained to `{"decision":"approve"|"block",
  "reason":"..."}`. Default timeout 30s (`DEFAULT_PROMPT_TIMEOUT_S`).
  Anything that fails to parse as the verdict shape is read as approval —
  "a hook that blocks the agent every time the model is chatty is a hook
  nobody keeps" (`hooks.cjs:430-433`).

Configuration merges **three sources**, in this order (later overrides
earlier for the same event+matcher, but all matching handlers across sources
run — it's additive, not override): workspace `hooks/hooks.json` → project
`<cwd>/.inertia/hooks.json` → `agent.hooks` on the agent record
(`hooks.cjs:182-216`). Read fresh every turn (a config edit takes effect on
the very next tool call, not the next app restart).

Handlers for one event run **in parallel** via `Promise.all` and cannot see
each other; outcomes are merged: any `block` wins; `decision` ranks
`deny(3) > ask(2) > allow(1)`, most-cautious wins; `context` strings
accumulate (all handlers' additional context is kept, not just the first);
`message` strings concatenate; `stop` is sticky (`hooks.cjs:629-646`). A
handler that crashes or times out is logged to the failure log and treated
as a non-blocking no-op — "a broken hook must cost the user the hook, not the
agent" (`hooks.cjs:33`).

A `PreToolUse` handler's `decision: "allow"` becomes `preapproved` fed into
the permission ask (step 4 of §3.6); `decision: "deny"` blocks the call
outright before `execute` ever runs; `decision: "ask"` (`forceAsk`) forces a
human prompt even if the static ruleset would have allowed it silently.

### 3.9 Termination conditions (exhaustive)

Every exit from `converse()` returns `{text, steps, stopped, usage,
transcript}`:

| `stopped` | condition | file:line |
|---|---|---|
| `"blocked"` | `UserPromptSubmit` hook blocked before any LLM call | `index.cjs:1074-1080` |
| `"complete"` | model stopped requesting tools, no pending steering, no live sub-runs, `Stop` hook didn't object | `index.cjs:1447-1473` |
| `"hook"` | a `PreToolUse`/`PostToolUse` hook returned `continue:false` | `index.cjs:1843-1888` |
| `"refused"` | user answered "reject" on a permission prompt | `index.cjs:1817-1897` |
| `"error"` | unrecoverable provider error, or a recovery budget was exhausted | `index.cjs:1355-1365` |
| `"cancelled"` | `signal.aborted` observed at any loop checkpoint | `index.cjs:1905-1911` |
| `"max-steps"` | the forced tools-off final step produced no further usable reply | `index.cjs:1916-1923` |

Bounded-retry sub-limits (not terminations themselves, but caps on how many
times the loop forgives before falling through to a real termination):

- `MAX_STEPS = 100` — hard step ceiling (`index.cjs:94`).
- `LOOP_THRESHOLD = 3` — identical-call detector; asks permission once, then
  the counter resets (`index.cjs:105`).
- `STOP_HOOK_LIMIT = 5` — how many times a `Stop` hook may send the model
  back in before it's allowed to actually stop (`index.cjs:78`).
- `MAX_BUILTIN_NUDGES = 2` — built-in-tool-refusal correction attempts
  (`index.cjs:177`).
- `CONTEXT_WARN_AT = 0.85` — not a termination; a one-time note to the model
  about approaching the context limit (`index.cjs:116`).
- context-overflow force-compaction and image-refusal recovery each happen
  **at most once per turn** (`recovered`, `blind` flags).

### 3.10 Cancellation / abort

- Every turn gets its own `AbortController` created in `ipc.cjs:308`,
  stored in a `running: Map<turnId, controller>`.
- `agent:cancel` IPC → `cancelTurn(id)` aborts and removes the entry
  (`ipc.cjs:579-585`). `agent:cancel-all` aborts every live controller and
  marks their store records `"cancelled"` — used when the window itself
  closes mid-stream (`ipc.cjs:770-779`).
- The one `AbortSignal` threads through the entire call chain: `session.run`
  → `converse` → every `await` boundary, including the provider stream
  itself.
- Checkpoints where `signal.aborted` is explicitly re-checked: top of every
  loop iteration (before and after the pause-gate await), immediately after
  `streamChat()` returns, before and inside `performCall`, at both ends of
  the batch loop, and at the very top/bottom of the outer `while`.
- **Mid-stream abort** is handled inside the transport itself: `attemptChat`
  in `anthropic.cjs` listens for the caller's `signal` and aborts its own
  internal `fetch` `AbortController`; a cancelled attempt resolves as
  `onEvent({type:"done", finish:"cancelled", usage:null})` rather than
  raising an error (`anthropic.cjs:429-430, 646, 717-720`).
- **A tool call in flight when abort fires** is caught specially and turned
  into a synthetic **failed tool result** ("The user stopped the turn before
  this finished.") rather than an uncaught exception — this keeps the
  transcript well-formed: no `tool_use` block is ever left without an
  answering `tool_result` (`index.cjs:1713-1731`; the Anthropic transport's
  own `pairToolCalls()` also independently guards this invariant when
  rebuilding messages, see §1.14/§6).
- **Pending permission questions** are rejected via an `abort` listener
  registered inside `permission.ask` itself
  (`permission/index.cjs:195-204`), turning into
  `PermissionDenied("The user stopped the turn.")`.
- **Final result on cancellation**: `{text: finalText, steps, stopped:
  "cancelled", usage: totals, transcript}` plus a `done` event carrying
  `stopped: "cancelled"`. Usage/cost already incurred is still reported —
  deliberately, since the tokens were already spent (`index.cjs:1905-1911`).
- **Steering** (`agent:steer` IPC → `session.steer(id, text)`) is the
  *non-destructive* alternative to cancellation: it enqueues text for the
  running turn (`createSteering`, `index.cjs:395-430`), taken only at the
  very top of the next loop iteration (§3.2 step 3) — never mid-batch, never
  between a tool call and its result.
- `run()`'s `finally` block (`index.cjs:489-505`) unconditionally: closes the
  steering queue, removes the turn from the `running` map, and — for a
  top-level turn only — cancels any sub-agent conversation tree it spawned
  (`crew.cancelConversation`), so aborting a parent turn tears down its
  whole team rather than leaving orphans billing quietly.

---

## 4. Sub-agents / crew

Two delegation shapes coexist, chosen for different situations
(`shared/crew.js:1-25`, `tools/builtin/crew.cjs:1-36`):

- **`task`** (`tools/builtin/task.cjs`) — blocking. Ask a question, wait for
  the answer. Right for "read forty files and tell me the answer" where
  those forty files should never enter the parent's own context.
- **`spawn` / `collect` / `wait` / `team` / `agent_send` / `interrupt` /
  `followup`** (`tools/builtin/crew.cjs`) — non-blocking. `spawn` starts a
  run and returns an id immediately; the parent keeps working and calls
  `collect` only when it actually needs the answer.

### 4.1 `task` (blocking delegation)

`tools/builtin/task.cjs:28-149`. Resolves an agent by id/name/handle from
the workspace's `agents` collection. Recurses into
`session.run({...})` directly (not through the crew run-table — `task` has
no row in the crew panel; it's a plain nested call).

Inherited from the parent `ctx`: `provider, apiKey, root, cwd, skills, depth
+ 1, unattended, approval, signal`. **Not** inherited: the parent's
conversation history — the sub-agent's `history` is built fresh from just
`args.prompt` (plus a stored continuation history if `task_id` names a prior
session, letting the same task session be asked follow-up questions cheaply
via an in-memory `sessions: Map<taskId, history[]>`). Model: the child
agent's own pinned model if it has an `agent.model` containing a `/`
(provider/model), otherwise the parent's current model — "a cheap agent
asked to do careful work quietly does it badly" (`task.cjs:101-103`).

A subagent **cannot itself call `task`** — enforced by the tool registry not
offering `task` past a depth threshold, not by a runtime check
(`task.cjs:14-16`).

Progress surfaces to the parent via `ctx.update({metadata: {taskId, agent,
state, steps}})`, fed by an `onEvent` callback watching for `tool-start`
events from the child's own loop (`task.cjs:114-121`) — this is how the
parent's UI shows "Reading 6 files..." for a task in flight even though the
call is `await`-blocking underneath.

Result returned to the parent as a tool result string, not structured data:
```
<task id="{id}" agent="{agentName}" state="{stopped}">
{result.text.trim()}
</task>
```
If the sub-agent produced no text at all (e.g. it errored), this throws a
`ToolError` telling the parent to retry with a more specific prompt or do
the work itself — never silently returns emptiness.

### 4.2 `spawn` (non-blocking delegation) → the crew run table

`tools/builtin/crew.cjs:106-390` (the tool), `team/crew.cjs` (the runtime
table, in-memory, main-process-only, never persisted to disk — "a run is a
live session with an abort controller and a promise attached, and neither of
those survives a restart", `crew.cjs:9-14`).

**Who may spawn what** (`shared/crew.js: spawnPolicy`, `spawnRefusal`):
per-agent policy `{subagents: bool, agents: bool, recursive: bool,
maxConcurrent: number}` (`SPAWN_DEFAULTS`: subagents on, everything else
off/unlimited by default). Layered checks, each producing a
model-actionable refusal string rather than a bare denial
(`spawnRefusal`, `shared/crew.js:116-141`):
1. conversation-wide total ever-started `>= TEAM_LIMITS.total (100)` → hard
   refuse.
2. conversation-wide currently-live `>= TEAM_LIMITS.live (20)` → refuse,
   suggesting `wait`/`collect` first.
3. `!policy.subagents` → this agent cannot delegate at all.
4. `depth > 0 && !policy.recursive` → a sub-run cannot itself spawn further
   sub-runs unless explicitly allowed.
5. spawning a **named existing agent** (`peer: true`) when
   `!policy.agents` → may only spawn temporary helpers, not full configured
   agents.
6. this agent's **own** live children `>= policy.maxConcurrent` (0 =
   unlimited) → refuse until some are collected.

**Target of a spawn**: either an existing configured agent (found the same
way `task` resolves one — by id/name/`@handle`), or (when `agent` arg is
omitted) a **temporary helper** built from just a `role` string — "no record
on disk, no computer and no permissions of its own... runs under the
parent's rules and disappears when the conversation does"
(`crew.cjs:68-91`). A temporary helper's spawn policy is **inherited from
its parent** rather than defaulted, specifically so a forbidden-to-nest
agent can't route around the restriction by making a helper and having the
helper delegate instead (`crew.cjs:76-80`).

**What a spawned run inherits from the parent** (`launch()`,
`crew.cjs:106-276`): `provider, apiKey, root, cwd, skills, depth+1, approval,
signal` (chained to the parent's own controller — killing the parent kills
the child). **`unattended: true` always** — a spawned run has no window
watching it, so anything that would normally stop to ask a person refuses
instead, the same way a routine does (§3.6 step 7). The run's `history`
starts as just `[{role:"user", content: brief}]` — **not** the parent's
transcript — unless `share_context: true` is passed, in which case the brief
is prefixed with a digest of the parent's recent steps
(`ctx.recentContext(SHARED_CONTEXT_CHARS=12000)`) wrapped in a
`<parent_context>` tag, while the tool description still tells the model to
"write the brief as if it did not [have that context]" (defensive
redundancy: the digest is a bonus, not a substitute for a self-contained
brief).

**Isolation**: `isolation: "worktree"` creates a git worktree
(`tools/builtin/worktree.cjs`) before the run starts (asking permission
first, since `git worktree add` is itself a shell-ish action) so the helper
edits/runs commands on a separate checkout+branch (`inertia/<name>`) without
touching the parent's working directory.

**Concurrency mechanics**: `spawn` does **not** await the child's
`session.run(...)` promise — it hands the promise to `crew.attach(runId,
{controller, promise, relaunch, followUp})` and returns immediately with
just the run id and a status message telling the model to do its own work
and call `collect` later (`crew.cjs:176-276`). This is "the entire feature":
starting work and needing its result are different moments, and the gap
between them is what the parent spends doing its own share.

Each run's promise, on settling, calls `crew.finish(runId, {result, usage,
transcript, generation})` or `crew.fail(runId, message, {cancelled,
transcript, generation})` (`crew.cjs:312-381`) — these update the shared
run table and call `notify(conversationId)` for the UI panel, and (on
`finish`) check `settledAll(conversationId)` to fire a one-shot
"team-finished" notice when the whole tree has settled.

**`collect(run_ids[], timeout_seconds)`** — awaits `Promise.all(ids.map(id
=> crew.settle(id, {timeoutMs})))`; **never rejects** — a failed helper
comes back as a `status: "failed"` row for the model to decide what to do
about, not a thrown error that ends the parent's own turn
(`crew.cjs:732-748`, `tools/builtin/crew.cjs:406-491`). All requested ids are
awaited together (`Promise.all`, not sequential) specifically so waiting on
the slowest one doesn't turn concurrent collection back into a queue.

**`wait(run_ids?, timeout_seconds)`** — blocks until *any* of: a watched run
settles, a message arrives in the parent's inbox, the user steers the turn,
timeout, or the turn is cancelled — whichever first
(`crew.waitFor`, `crew.cjs:622-668`). Built on the same `onChange` listener
set the UI panel uses, so it costs nothing between events (no polling).
Returns immediately, saying so, if there's nothing running and nothing
waiting.

**`team()`** — synchronous snapshot: every run in the conversation with
status/activity/step-count, plus any unread inbox messages, never blocks.

**`agent_send(to, message)`** — fire-and-forget mailbox post
(`crew.post`, `crew.cjs:793-808`); `to: "parent"` resolves to
`ctx.parentRunId`. Delivered to the target's `inbox`, read the next time
*that* run calls `team()` — never interrupts a run mid-thought.

**`interrupt(run_id, reason)`** — stops a run's *current turn* but **keeps
its transcript and row** (status becomes `"interrupted"`, distinct from
`"cancelled"`). Its own children are left running (they were doing
independently-useful work). This is for "the brief has changed under a run"
where a mailbox message would arrive too late (`crew.cjs:533-549`).

**`followup(run_id, prompt)`** — only valid on a `canFollowUp` status
(`done | failed | interrupted`, `shared/crew.js:159-161`) whose transcript is
still held in-memory (`transcripts: Map`) and whose `relaunchers` closure
survived (both are lost if the app restarted since the run finished). Reuses
the **same row** (`crew.reopen`, bumping `generation` so any still-unwinding
promise from the previous attempt is recognized as stale and ignored,
`crew.cjs:552-576, 252-254`), replays the stored transcript plus the new
prompt — genuinely cheaper than a fresh `spawn` because the helper doesn't
re-read what it already read.

**Pause/resume** (`crew.pause/resume`, `crew.cjs:398-479`): a "gate" the
loop's `waitWhilePaused()` call blocks on at the top of its next iteration —
"there is no way to freeze [a request] mid-flight... so a pause here is a
gate the loop waits at, and it takes effect at the next boundary." Pausing a
run pauses its whole subtree by default (a helper whose coordinator is
paused would otherwise keep working toward a report nobody reads).

**Cancellation of a tree**: `crew.cancel(id, {descendants: true})` aborts
the run's own controller **and recursively cancels every child**, because an
orphaned child has nobody to report to and would keep spending money
uselessly. `cancelConversation(conversationId)` is what a top-level turn's
own cancellation calls to tear down everything it spawned.

**Mid-turn completion notification** (the "you will be told when a run
finishes" mechanism from the `teamwork()` prompt section, §1.8): the main
loop's completion branch (§3.2 step 12) checks `crew.activeChildren` and, if
any of the parent's own direct children just settled since the last check,
injects `settledNotice(runs)` (`shared/crew.js:216-243`) as a synthetic user
note into the transcript **before** the next model request — this is what
lets the parent "find out" about a finished helper without polling `team()`
in a loop.

### 4.3 Group / floor turn-taking

Group conversations are a **separate subsystem** from the crew/spawn
mechanism above, deliberately (`team/group.cjs` header): crew is "a helper
works in its own session and reports back"; a group room is "several agents
in *one* conversation, reading the same transcript, no session per agent, no
transcript copying." The turn-taking *protocol* the model is told about is
prompt-driven (§1.9, the `@handle` convention) but *which agent actually
gets dispatched next* is decided by deterministic host-side code, not by the
model calling a tool.

**`team/floor.cjs`** — pure function computing the next speaker, no I/O:

- `openFloor(primary)` → `{active: primary, queue: [], hops: 0, reason:
  "start"}` (`floor.cjs:26-28`).
- `afterPerson(floor, {roster, primary, mentioned})` (`floor.cjs:46-56`): if
  the person's message `@`-mentioned agents present in the roster, they go
  first in the order named, the rest queued, `hops` reset to 0. Otherwise
  falls back to, in order: whoever was mid-turn (`floor.active`) → whoever
  spoke last (`floor.last`) → the primary agent → the first roster entry —
  picking the first of these still present in the roster. This is what makes
  an unaddressed message "to the room": everyone still eventually gets a
  turn via the queue.
- `afterAgent(floor, outcome, {roster, primary, maxHops, mentioned,
  speaker})` (`floor.cjs:79-122`) — called once an agent's reply finishes
  streaming:
  - `outcome.kind` is one of `handover | invite | leave | done`, set by the
    `invite`/`handover`/`part` tools via `room.pending` (consumed here, not
    applied immediately when the tool is called).
  - Builds an `asked` list: the tool-directed target first (whoever was
    handed-over-to or invited), then any `@mentioned` names parsed out of
    the reply text, self-mentions excluded.
  - **The person's own queued list always outranks what the agent just
    asked for** — a person's `@mention` from their last message is not
    overridden by an agent deciding to talk to someone else.
  - `hops` increments on every agent-to-agent turn; once `hops >=
    MAX_HOPS = 8` (`shared/group.js:11`), the floor is forced back to the
    person regardless of what the reply asked for (`reason: "hop-limit"`)
    — this is the hard backstop against an infinite agent-to-agent ping-pong
    that never lets the human back in.
  - Otherwise: `handover`/`invite`/mention → hands to that agent; `leave` →
    floor returns to whoever brought the leaving agent in (or the primary);
    anything else (a reply naming nobody) → floor returns to the person,
    `reason: "done"`.
- `join`/`part` (`floor.cjs:137-163`) — roster add/remove, capped at
  `MAX_AGENTS = 6` (`shared/group.js:14`); the primary agent can never
  leave.

**`team/group.cjs`** — stateful wrapper around `floor.cjs`, one `Room` per
`conversationId`. `open(conversationId, {primary, permissions, roster})` is
re-run at the top of **every** group turn (not cached once) since nothing
guarantees the main process was alive when the room was first created; it
re-seats on a changed primary without wiping the existing roster.
`invite`/`handover`/`leave` are the runtime behind the `invite`/`handover`/
`part` tools: each checks the room's own `{canInvite, canHandover,
canLeave}` permissions (all default `true`), and — unless called directly by
the person — sets `room.pending = {kind, agentId}` for `floor.afterAgent` to
consume on the *next* `spoke()` call rather than switching immediately.
`asked(conversationId, {mentioned})` runs when the **person** speaks (pulls
in newly-mentioned agents, clears `pending`, calls `floor.afterPerson`);
`spoke(conversationId, agentId, {mentioned})` runs when an **agent's** turn
ends (reads `pending`, resolves any newly-`@mentioned` names into invites if
permitted, calls `floor.afterAgent`); `resume(conversationId, agentId)` is
used for a Retry action — hands the floor straight back to a named agent
without treating it as a fresh question.

**Wiring into the turn dispatcher** (`src/main/session/ipc.cjs`):
`seatRoom()` is called at the top of every group-mode send, calls
`group.open(...)`, then either `group.resume(...)` (a Retry, when
`request.speaker` is set), `group.asked(...)` (a fresh message from the
person, when there's no continuation link), or leaves room state untouched
(mid-chain continuation). It returns the resolved `speaker` id plus a
`describe` block (`me`, `person`, `roster`, `permissions`, `custom`) that
becomes the `group` argument to `prompt.build()` (§1.9) — **not** the raw
transcript of what other agents have internally reasoned about, only who
they are.

**Per-seat transcript rewriting** — `team/perspective.cjs`. Before a
request is sent, the *one shared* transcript is rewritten through
`perspective(history, {me, names, person})` so each seated agent sees
itself as `assistant` and **everyone else — the person included — as
`user`-role lines prefixed with a name**. A colleague's tool calls/results
are folded into a single compact line (`(ran toolname args)\nresult`,
clipped to `RESULT_LIMIT = 4000` chars) rather than replayed in full. This
exists specifically to stop a model from writing a whole scripted
multi-voice dialogue under its own turn — without it, an agent that can see
raw `assistant`-role turns from colleagues starts drafting lines *for* them.
**This per-seat rewrite is the mechanism that makes a shared-transcript room
work with providers that only understand a strict user/assistant
alternation** — a Rust port must reimplement this rewrite, not attempt to
send the raw shared transcript with multiple "assistant" identities to the
provider.

**Closing the loop after a stream**: once a seated agent's reply finishes
streaming, `mentionedAgents(text, roster)` (`shared/group.js`) scans the
text for `@slug` patterns against every roster member's `slugOf(name)`, in
order, deduplicated. `group.spoke(conversationId, speakerId, {mentioned})`
is called with that list — deliberately **before** the turn's own `done`
event is allowed to reach the renderer, so the window's next render already
reflects the new floor state — and a fresh room snapshot is pushed to the
UI. This snapshot (specifically, whose turn is now active) is what triggers
the dispatcher to actually start the next agent's turn.

`invite`/`handover`/`part` (the `GROUP_ONLY` tool set, §1.5) are the only
tool-mediated moves; everything else about who speaks next is host-decided
from the `@mention` text and the `MAX_HOPS` backstop, not requested by a
dedicated "pass the floor" tool call.

---

## 5. Events emitted to the UI (event contract)

Two layers: **transport-level** events from `llm/anthropic.cjs` /
`llm/openai.cjs` (consumed only by the session loop, not IPC), and
**loop-level** events emitted by `session/index.cjs` via `onEvent(...)`,
which `session/ipc.cjs` wraps and forwards to the renderer over Electron
IPC. For a Tauri port, the loop-level events below are the ones to emit
as Tauri events (`emit("agent:event", payload)` equivalent).

### 5.1 Transport-level (`onEvent` inside `streamChat`, both providers emit the same set)

| type | payload | meaning |
|---|---|---|
| `start` | `{type, model}` | provider accepted the request, streaming begins |
| `delta` | `{type, text}` | reply text token(s) |
| `reasoning` | `{type, text}` | extended-thinking token(s) |
| `thinking` | `{type, blocks}` | assembled signed thinking blocks (Anthropic only), must be replayed verbatim next request |
| `tool` | `{type, calls: [{id, name, arguments}]}` | one or more tool_use blocks finished streaming |
| `notice` | `{type, message}` | non-fatal degrade-ladder note (e.g. "this endpoint doesn't support the hour cache TTL") |
| `retry` | `{type, attempt, of, delayMs, status, message}` | about to retry after a transient failure |
| `error` | `{type, message, status}` | unrecoverable transport failure |
| `done` | `{type, finish, usage}` | stream ended (`finish` includes `"cancelled"` on abort) |

Source: `llm/anthropic.cjs:512-637` (Anthropic SSE handling),
`llm/openai.cjs` (equivalent shape for chat-completions SSE).

### 5.2 Loop-level (`session/index.cjs` → `session/ipc.cjs` → IPC channel `"agent:event"`, payload always carries `{..., id, threadId}`)

| type | payload | file:line |
|---|---|---|
| `delta` | `{type, text}` | `index.cjs:1222` |
| `reasoning` | `{type, text}` | `index.cjs:1224` |
| `warning` | `{type, kind?, message, problems?}` — provider notices, retries, context-usage/compaction notes, hook messages, refusal notes | `index.cjs:729, 835, 1196, 1246, 1310, 1465, 1847, 1892-1897` |
| `usage` | `{type, step, usage, totals, context: {used, window, estimated, pruned, compacted}}` | `index.cjs:1262-1274` |
| `error` | `{type, message, status?, usage?}` | `index.cjs:1078, 1364` |
| `step` | `{type, index, calls}` — one loop iteration about to run N tool calls | `index.cjs:1476` |
| `tool-start` | `{type, callId, name, title, args}` | `index.cjs:1567, 1597, 1615-1621` |
| `tool-update` | `{type, callId, ...partial}` — mid-run progress (used heavily by `task`/`spawn` to surface sub-agent step counts) | `index.cjs:1707` |
| `tool-end` | `{type, callId, name, ok, title?, output, metadata?, durationMs?}` | `index.cjs:1522, 1568, 1598, 1791-1800` |
| `changes` | `{type, from, to, cwd, files}` — filesystem diff produced by the turn (for a change-review UI) | `index.cjs:818` |
| `steer` | `{type, text}` — a mid-turn message was accepted into the queue | `index.cjs:404` |
| `hook` | `{type, ...hookRunSummary}` — one hook handler finished | `index.cjs:873` |
| `done` | `{type, usage, steps, stopped?}` | `index.cjs:1447, 1472, 1886, 1896, 1909, 1922` |

Two additional IPC channels, separate from `"agent:event"`, both driven by
their own notifier callbacks rather than the loop's `onEvent`:

- **`"permission"`** channel — `{type:"asked", question}` /
  `{type:"settled", id}`, from `permission.setNotifier(...)`
  (`permission/index.cjs:44-47, 189, 201, 245, 262, 273`). This is how a
  pending permission card reaches the renderer, and how it's told to remove
  one that settled (answered, or its turn was aborted).
- **`"question"`** channel — `{type:"asked", question}`, from the `question`
  built-in tool's own notifier (`tools/builtin/question.cjs:108`) — the
  `question` tool is a *different* mechanism from permission prompts: it's
  the model explicitly asking the user to choose between options, not a
  permission gate.

A group conversation additionally emits an ipc-level (not loop-level)
`{type:"group", id, threadId, speaker, modelRef?, room}` event
(`ipc.cjs:335-347, 372-377`) when the active speaker changes.

**Port note:** a Rust/Tauri implementation should keep this same
three-channel split (`agent:event`, `permission`, `question`) — collapsing
permission/question into the generic event stream would couple UI-prompt
lifecycle to turn lifecycle in a way the source deliberately avoids (a
permission card must be able to settle — via `"settled"` — independently of
whether the turn that asked it is still running, e.g. on abort).

---

## 6. Rust design notes

### 6.1 Module split

Mirror the source's separation of concerns as separate crates or modules:

- **`prompt`** — pure function(s) building the system-prompt string from a
  `PromptInput` struct (agent record, env facts, workspace facts, profile,
  connected apps, mode, approval, group room, skills list, memories list).
  No I/O beyond reading `AGENTS.md`-family files and the two static prompt
  text assets (which can be `include_str!`'d). Should be trivially unit
  testable exactly the way `prompt.test.js` tests it — one function per
  section, each independently callable and independently nullable.
- **`permission`** — the pure rule-evaluation engine (`shared/permission.js`
  equivalent: glob matching, specificity ranking, merge, evaluate) as a
  standalone crate with zero I/O, usable by both the settings UI and the
  enforcement path. A second, stateful `permission_broker` module (mirroring
  `main/permission/index.cjs`) owns the pending-question map, the
  session-scoped "always" grants, and the notifier channel.
- **`hooks`** — config loading/merging (3 sources), matcher evaluation,
  command/prompt execution, outcome merging. Should not know about the loop
  internals; exposes `fire(event, ctx) -> Outcome`.
- **`tools`** — the `Tool` trait + `run_tool()` pipeline (normalize →
  validate → permission-ask → execute → truncate). Each built-in tool is a
  separate module implementing the trait; MCP/OpenAPI/Composio sources
  implement the same trait via adapters.
- **`crew`** — the in-memory run table (`RunTable` struct wrapping a
  `HashMap<RunId, Run>` behind a mutex or actor), spawn-policy checks, the
  `wait_for`/`settle`/`pause`/`interrupt`/`follow_up` operations. Should own
  no provider/LLM knowledge — it's given an opaque "start a run" closure by
  its caller, exactly as `launch()` in the JS source takes `session.run` as
  a dependency rather than importing it eagerly (worth preserving to avoid a
  circular-module problem, which is literally why the JS does a lazy
  `require` at the top of `launch()`).
- **`llm`** — one transport trait `ChatTransport { fn stream_chat(...) ->
  impl Stream<Item=TransportEvent> }` with `AnthropicTransport` and
  `OpenAiTransport` (or a generic OpenAI-compatible transport) implementers.
  Keep `context::compact()` and `cache::with_cache_control()` as pure
  functions independent of the transport, exactly as the source does
  (`llm/context.cjs`, `llm/cache.cjs` have zero network I/O).
- **`session`** (or `turn`) — the state machine itself, described in §3.
  This is the piece most naturally modeled as an explicit state enum plus a
  driver loop (see §6.2).

### 6.2 Modeling the loop: explicit async state machine, not an actor

The source's loop is a single `while` loop with early returns, not a
message-passing actor — and this is a reasonable shape to keep in Rust,
because:

- The state that needs to persist across iterations (transcript, step
  count, recovery flags, steering queue reference) is small and owned by one
  task; there's no need for actor-style isolation between iterations of the
  *same* turn.
- Concurrency exists **between** turns (sub-agent runs) — that's what the
  crew module is for — and **within** a batch of tool calls (`join_all` over
  independent tool executions), not within the turn's own control flow.

Recommended Rust shape: an `enum TurnState { Setup, Requesting, Streaming,
ExecutingTools(Vec<ToolCall>), Compacting, Done(StopReason), ... }` is
optional polish; the more important thing to preserve is the **exact
ordering of checks** documented in §3.2 (pause gate → abort check → steering
fold-in → settled-subagent note → context-warning note → last-step
tools-off → pre-request compaction → provider call → post-stream abort
check → error-recovery ladder → completion/tool-dispatch branch). A `loop {
... }` with early `return`s for each `stopped` variant, driven by `tokio`
for the async provider call and tool execution, maps very directly onto the
source and is easier to verify against it line-by-line than a more
"idiomatic" state-machine encoding would be. Use a `CancellationToken`
(tokio-util) or a `watch<bool>` channel as the `AbortSignal` equivalent,
checked at the same points enumerated in §3.10.

Tool-call batching within one assistant turn: `futures::future::join_all`
(or `try_join_all` if a single hard failure should short-circuit — but note
the source does **not** short-circuit on tool failure, it lets each
`performCall` resolve to an `ok:false` result and keeps going unless the
failure was specifically a user-refused permission, §3.4). Preserve that
distinction: a tool error is data, a user refusal is a batch-stopping
signal.

### 6.3 Trait seams for testability (providers / tools / permission)

To keep the same "mock everything, test the state machine in isolation"
property the JS test suite relies on (`anthropic-loop.test.js` drives the
loop through a fake HTTP server speaking the wire format rather than mocking
internals):

- **`ChatTransport` trait**: `async fn stream_chat(&self, req: ChatRequest,
  sink: impl Sink<TransportEvent>, cancel: CancellationToken) ->
  Result<()>`. A test-double implementation can be scripted to emit a fixed
  sequence of `TransportEvent`s (including `tool` calls and `error`s) without
  any network I/O, mirroring how the JS tests stand up a real local HTTP
  server that speaks the Anthropic SSE format — either approach (trait mock,
  or a local test server) is valid in Rust; the trait gives you the option
  of not needing a server at all for pure-loop tests.
- **`Tool` trait** (§6.1): each tool is independently unit-testable with a
  fake `Ctx` (fake filesystem root, fake permission asker that always
  allows/denies/prompts a fixed sequence). The `run_tool()` pipeline itself
  (normalize/validate/permission/execute/truncate) should be tested once,
  generically, against a handful of dummy tools — exactly as
  `tools/CONTRACT.md` documents the pipeline as tool-agnostic.
- **`PermissionAsker` trait**: `async fn ask(&self, req: AskRequest) ->
  Result<Verdict, PermissionDenied>`. The stateful broker
  (§6.1 `permission_broker`) implements this against a real pending-question
  channel; tests substitute a scripted asker (always-allow, always-deny, or
  a fixed sequence of answers) to drive the loop through every branch in
  §3.6 without a UI.
- **`CrewHandle` trait** given to the `spawn`/`collect`/... tool
  implementations, abstracting over the run table so those tools can be
  tested against an in-memory fake table without pulling in the real
  provider-calling `launch()` closure.
- Keep `prompt::build()` pure and synchronous-except-for-file-reads so it
  can be golden-tested against fixed input structs the way
  `prompt.test.js` does — no hidden global state (the JS source's only
  global-ish state is the `cached` static-text memoization at
  `prompt.cjs:41`, which is safe to mirror with a `once_cell`/`OnceLock`
  holding the two static prompt strings).

### 6.4 Things easy to silently get wrong in a port (explicit warnings)

- **Section omission, not empty rendering.** Every prompt section function
  must return `Option<String>` (or equivalent), and a `None` must be
  entirely absent from the joined output, not an empty heading. Several
  sections are tested specifically for this (`prompt.test.js`).
- **Tool-answer pairing invariant.** Any transcript representation must
  guarantee every `tool_use` has exactly one matching `tool_result` before
  it is ever sent to Anthropic, including after a mid-flight abort — the
  source enforces this in *two* independent places (the loop's own abort
  handling, §3.10, and the Anthropic message converter's `pairToolCalls`,
  §1.14) precisely because getting it wrong is a silent-until-it-happens
  400 that kills a turn. A Rust port should probably enforce it in one
  place only (the transcript type itself, e.g. a `push_tool_call` that
  can't compile/construct an unanswered dangling call across a serialize
  boundary) rather than duplicating the guard.
- **"One no cancels the whole batch"** (§3.4, §3.6) is easy to miss if
  tool-call results are naively collected with `join_all` and only
  individually reported — the *batch* must observe a `refused` result and
  stop dispatching further calls in that batch, not just record the
  refusal.
- **Approval dial vs. conversation mode are two independent axes** (§1.5 vs
  §1.6) — do not conflate "may this turn see this tool" (mode) with "does
  this tool stop to ask" (approval dial). Both must be threaded separately
  into prompt-building and into tool-list filtering.
- **Session-scoped permission grants must not leak into persisted
  settings**, and must not survive a new conversation (`permission.forget`).
- **`unattended` (routine) turns always deny permission asks**, never queue
  a UI prompt — this is a correctness requirement, not an optimization: a
  routine with nobody watching that somehow displayed a UI prompt would hang
  forever.
