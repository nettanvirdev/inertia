# 06 — Cross-cutting subsystems: port specification

Source of truth: `D:\oss\inertia\src\main\{permission,hooks,routines,sandbox}`,
`D:\oss\inertia\src\main\{background,notify,failures,security,log}.cjs`, and
`D:\oss\inertia\src\shared\{permission,approval}.js`. All line numbers refer to
those files as they exist at the time of writing. This document is read-only
research — nothing under `src/main` was modified.

---

## 1. Permissions

The permission system is split across two layers that must both be ported:

- **`src/shared/permission.js`** (pure, dependency-free, 300 lines) — the rule
  engine: what a ruleset says about a given tool call. Shared between renderer
  (for editing) and main (for enforcement) in the Electron app; in a Tauri port
  this becomes a single Rust crate/module used by both the backend and any
  settings UI logic.
- **`src/main/permission/index.cjs`** (344 lines) — the "ask" half: turns a
  verdict of `ask` into a suspended tool call, a card shown to a person, and a
  resolved/rejected promise. This is inherently async/stateful and is the part
  that needs the most careful concurrency design in Rust.

### 1.1 Core vocabulary

- **Actions**: `"allow" | "ask" | "deny"` (`permission.js:20`). Default action
  when no rule matches is `"ask"` (`DEFAULT_ACTION`, line 22).
- **`ANY = "*"`** — a rule with no explicit target/pattern covers every
  argument to that tool (line 25).
- **A rule** is `{ tool: string, pattern: string, action: "allow"|"ask"|"deny" }`
  (`rule(tool, action, pattern = ANY)`, line 72). Rulesets are **flat, ordered
  arrays**, never nested — deliberately, "so it is edited as a list, diffed as
  a list, and merged as a list" (comment at line 66-71).
- **Pattern matching** (`matches(pattern, value)`, line 36): restricted glob
  syntax using only `*` and `?` (no full regex, by design — "a rule the author
  cannot predict the meaning of is worse than no rule"). `*` → `.*`, `?` → `.`,
  everything else is regex-escaped, anchored `^...$`, with the `s` (dotall)
  flag.
- **Specificity** (`specificity(pattern)`, line 56): used to rank multiple
  matching rules. `ANY` = 0. Otherwise `length - wildcards*2`, plus **+1000
  bonus if the pattern has zero wildcards** (an exact literal always beats any
  wildcard pattern regardless of length).

### 1.2 Evaluation (`evaluate(rules, tool, target = ANY)`, permission.js:239)

For every rule in the ruleset:
1. Reject if `!matches(entry.tool, tool)` — **the tool name itself is a
   glob pattern**, e.g. a rule `mcp_*` can cover a whole family of dynamically
   discovered MCP tool names.
2. Reject if `!matches(entry.pattern, target)`.
3. Score = `specificity(entry.tool) * 100_000 + specificity(entry.pattern)`.
   **The tool match dominates**: any rule that names the exact tool outranks
   any rule matching the tool via wildcard, regardless of how specific the
   argument pattern is.
4. Keep the highest-scoring rule; ties keep the first found (later ones don't
   overwrite on equal score, since `score > bestScore` is strict).

Returns `{ action, rule, implicit }` where `implicit: true` means nothing
matched and the default `"ask"` was used (so a UI can distinguish "explicitly
set to ask" from "nobody configured this").

**`isDenied(rules, tool, target)`** — convenience wrapper, `action === "deny"`.

**`visibleTools(tools, rules)`** (line 284): filters which tools are even
*offered* to the model. A tool is hidden entirely only if it is denied for
*every* argument (`verdict.action === "deny" && (verdict.rule?.pattern ??
ANY) === ANY`) — i.e. an unconditional deny rule. A tool denied only for some
arguments (e.g. `shell: rm -rf *` → deny) stays visible because the model can
still call it correctly for other inputs.

**`toolsThatAsk(tools, rules)`** (line 294): tools whose verdict for `ANY` is
`"ask"` — used to summarize to the user "what will this agent check with me
about."

### 1.3 Config shapes and serialization

Three interchangeable shapes a person or file might contain, all normalized
through `evaluate`'s flat rule array:

1. **Shorthand config** (`fromConfig`/`toConfig`, lines 85-122):
   ```
   "deny"                              → [{tool:"*", pattern:"*", action:"deny"}]
   { shell: "ask" }                    → [{tool:"shell", pattern:"*", action:"ask"}]
   { shell: { "git push *": "ask" } }  → [{tool:"shell", pattern:"git push *", action:"ask"}]
   ```
2. **Flat/dotted form** (`fromFlat`/`toFlat`, lines 129-161) — used inside an
   agent's markdown frontmatter, where nesting is avoided. Key syntax:
   `tool` or `tool/pattern` (first `/` splits; `FLAT_SEPARATOR = "/"`). E.g.
   `shell/git push *: ask`.
3. **Array-of-objects** — the canonical wire/storage shape,
   `asRules(value)`/`asRuleMap(value)` (lines 179-200). Tolerant reader: bad
   entries (`entry.tool` not a non-empty string) are silently dropped; unknown
   actions normalize to `DEFAULT_ACTION` (`normalizeAction`, line 124).
   `asRuleMap` reads the `{ agentId -> ruleset }` shape for per-agent rules.

**Merge precedence** (`merge(...rulesets)`, line 209): later rulesets in the
argument list win on exact `(tool, pattern)` collision. Order used
consistently across the app: **defaults → workspace → agent's own**, so an
agent can tighten/loosen what the workspace said and the workspace can move
off defaults, with no layer needing to know the others exist. Dedup key is
`` `${tool}\u0000${pattern}` `` — a NUL-separated composite key chosen because
NUL cannot appear in either half (historical note: it used to be a literal NUL
byte in source, invisible in diffs — a footgun the comment explicitly warns
against reproducing).

### 1.4 The approval dial (`src/shared/approval.js`)

A **per-conversation** modifier, orthogonal to permission rules and to
"conversation mode" (chat/plan/autonomous, which controls *which tools are
held at all* — out of scope of this doc). Three positions
(`APPROVALS`, lines 35-51):

| id      | label         | effect |
|---------|---------------|--------|
| `ask`   | Ask           | rules as written (`DEFAULT_APPROVAL`) |
| `edits` | Accept edits  | file edits in the working folder auto-allowed; everything else still asks |
| `auto`  | Never ask     | anything a rule would ask about is allowed |

**`resolveAsk(approval, key)`** (line 91) — the only function that matters at
call time:
```
if ALWAYS_ASK.includes(key): return "ask"          // key === "delete_everything" — no dial position can waive this
if approval === "auto":       return key === "doom_loop" ? "deny" : "allow"
if approval === "edits" && EDIT_KEYS.includes(key): return "allow"   // EDIT_KEYS = ["edit"]
else: return "ask"
```
- `ALWAYS_ASK = ["delete_everything"]` — hard-coded floor; no dial position
  can waive it.
- `"doom_loop"` under `auto` resolves to **deny**, not allow — this is the
  repeated-identical-call loop guard (see §1.5): with nobody to ask and the
  dial saying "never ask," the only safe answer to "should I do the same
  thing a 4th time" is no.
- The dial can only **loosen an `ask` verdict into `allow`**. It can never
  override a `deny` from the rules (checked earlier in `permission/index.cjs`
  and thrown before the dial is even consulted).
- `promptForApproval(approval)` (line 107) generates system-prompt text
  telling the model which regime it's under, so it writes reviewable diffs
  under `ask` and self-verifies under `auto`.

### 1.5 The ask/allow/deny/always-allow flow — `permission/index.cjs`

**State** (all in-memory, per-process, session-scoped — explicitly *not*
persisted to disk on grant):
- `pending: Map<id, {question, resolve, reject}>` — in-flight questions.
- `granted: Map<sessionId, Rule[]>` — "always" grants, session-scoped. Comment
  explicitly rejects writing these to the workspace file: "A grant made in the
  middle of one conversation should not silently become a permanent setting
  the user never visited a settings screen to make."
- `nextId` — monotonic counter for `ask-${n}` ids.
- `notify: fn|null` — set once at startup (`setNotifier`), the renderer push
  channel.

**`rootSession(sessionId)`** (line 61): a subagent's session id is composite,
`"parent/task-3"`. Questions from a subagent must reach the *parent's* window
— `rootSession` strips to the first `/`-segment. Grants and teardown stay
scoped to the exact child id, but "whose screen does this belong on" always
resolves through `rootSession`.

**`ask(request)` — the full decision cascade**, in exact order
(`permission/index.cjs:82-206`):

1. Merge rules: `permission.merge(request.rules ?? [], grantsFor(sessionId))`
   — request-supplied rules (defaults+workspace+agent, already merged
   upstream) plus this session's "always" grants, session grants winning on
   collision.
2. `verdict = permission.evaluate(rules, key, target)`.
3. **If `deny`** → throw `PermissionDenied` immediately, with a message
   naming the specific rule (`` `Your rule \`${tool}${pattern===ANY?"":": "+pattern}\` forbids it.` ``)
   and instructing the model not to retry. This check is unconditional and
   happens before anything else, including hooks and the approval dial — deny
   always wins.
4. **If `allow`** and not `request.forceAsk` → return `{granted:true, asked:false}`
   immediately (fast path, most calls).
   - `forceAsk` is set by a `PreToolUse` hook that wants a human to look even
     though the rules would auto-allow — the inverse of a hook pre-approving.
5. **If `request.preapproved`** → return `{granted:true, asked:false, hook:true}`
   — set when a `PreToolUse` hook already answered "allow" for this exact call
   upstream (see §2).
6. **Approval dial**: `waved = resolveAsk(request.approval, key)`.
   - `waved === "allow"` and not `forceAsk` → return granted, `asked:false`,
     tag `approval: request.approval`.
   - `waved === "deny"` → throw `PermissionDenied` — this is the **doom-loop
     guard**: message says the same call was made
     `request.metadata?.repeats ?? "several"` times in a row and the model
     must not repeat it.
7. **`PermissionRequest` hook** (`request.beforeAsk`, a function): called with
   `{key, target, always: always ?? target, title: title ?? key}`, wrapped in
   try/catch (any throw → treated as no answer). Runs *before* the unattended
   check (step 8) intentionally — "a routine with a hook that decides its
   permissions is exactly the unattended run that should not have to refuse
   everything."
   - `answered?.decision === "allow"` → granted, `hook:true`.
   - `answered?.decision === "deny"` → throw `PermissionDenied` with the
     hook's `reason`.
   - Anything else → falls through.
8. **`request.unattended`** (routine turns) → throw `PermissionDenied` with a
   message explaining nobody can answer now, telling the model to finish the
   rest of the work and summarize what it wanted so a person can grant it
   later in the routine's agent permissions.
9. **`!notify`** (no window registered at all — headless) → throw
   `PermissionDenied`: "there is nobody to ask right now." Refusing is the
   only safe default for a headless run.
10. **`signal?.aborted`** (turn already cancelled) → throw immediately, don't
    put up a card for a dead turn.
11. **Otherwise**: build a `question` object and return a `Promise` that:
    - Registers `pending.set(id, {question, resolve, reject})`.
    - Calls `notify({type:"asked", question})`.
    - Registers a **one-shot abort listener** on `signal` that, if it fires
      while still pending, deletes from `pending`, rejects with
      `PermissionDenied("The user stopped the turn.")`, and notifies
      `{type:"settled", id}`. This is explicitly called out as a bug fix:
      without it, a cancelled turn's promise never resolves and the tool call
      (and thus the whole turn) hangs forever.

**Question payload shape** (exact fields, lines 171-185):
```js
{
  id: "ask-${n}",
  sessionId,
  agent,              // whose subagent this is, for the UI to attribute it
  key,                // the permission key / tool id
  target,             // the specific argument being evaluated (defaults to ANY)
  always: always ?? target,  // what an "always" grant would remember — shown on the button
  title: title ?? key,       // human label
  metadata,           // caller-supplied extra context (e.g. repeats count)
  at: Date.now(),
}
```

**`reply(id, answer, message = "")`** (line 216) — the person's verdict,
`answer ∈ {"once", "always", "reject"}`:
- Not found in `pending` → `{ok:false, reason:"That question is no longer waiting."}`.
- **`"reject"`**: builds a `PermissionDenied` (with `message` embedded if
  given: `` `The user refused this tool call and said: ${message}` ``),
  flags it `refusal.refusedByUser = true` (distinguishes a human's "no" from
  a rule's "no" — the loop treats them differently: a rule is worked around,
  a person's refusal ends the turn), rejects this entry, **then calls
  `rejectSession(sessionId, ..., {refusedByUser:true})`** — this is the
  documented "one no cancels the whole batch" behavior: every other pending
  question in the same session is rejected too, all tagged
  `refusedByUser: true`.
- **`"always"`**: appends `permission.rule(question.key, "allow", question.always)`
  to `grantsFor(sessionId)`, resolves this entry with
  `{granted:true, asked:true, remembered: question.always}`, then calls
  `settleCovered(sessionId)` — **re-evaluates every other still-pending
  question in the session against the updated grants** and auto-resolves any
  now covered by the new rule (without prompting again).
- **`"once"`**: resolves `{granted:true, asked:true}`, no persistence.

**Other exports**:
- `waiting(sessionId)` — every pending question whose `rootSession` matches,
  for a reopened window to re-render the queue.
- `allGrants()` — flat `{sessionId, tool, pattern, action}` rows across all
  sessions, for a settings screen.
- `revoke(sessionId, tool, pattern)` — removes one grant without touching
  in-flight questions (distinct from `forget`).
- `forget(sessionId)` — drops all grants for a session **and** calls
  `rejectSession` to refuse everything still pending: "A new conversation
  starts clean."
- `PermissionDenied` — `Error` subclass, `name = "PermissionDenied"`,
  `retryable = false`, optional `.rule` and `.refusedByUser` fields. This is
  the exception type the agent loop must special-case: message text is
  written *for the model* to read and act on (never retry blindly), and
  `refusedByUser` is what tells the loop to end the turn rather than let the
  model route around it.

### 1.6 Where rules persist

- **Workspace-level rules**: `hooks`/permission config lives in workspace
  collections (see `collections.readDocument`/`writeDocument` pattern used
  throughout — JSON documents under the chosen workspace root). Exact
  document names weren't in the files read for this doc but the pattern
  mirrors `hooks/hooks.json` (§2) — expect a `permissions`/`settings.*`
  document read via `collections.readDocument(root, DOCUMENT, default)`.
- **Per-agent rules**: `agent.permissions` (or similar field) parsed via
  `permission.asRules`/`asRuleMap`, merged in as the third/most-specific layer.
- **"Always" grants**: **never persisted** — in-memory `Map`, cleared on
  `forget(sessionId)` (conversation end) and lost on app restart by design.

### 1.7 Precedence summary (highest to lowest, first match wins)

1. A rule that evaluates to `deny` — unconditional, cannot be overridden by
   hooks, the approval dial, or "always" grants (a deny rule pre-empts an
   `evaluate` call from ever producing `allow` for that pattern in the first
   place, since `merge` lets a session's "always: allow" grant only add a new
   rule — it does not remove the workspace's `deny` rule, and `evaluate`'s
   specificity scoring decides the winner. **A person cannot "always allow"
   past a `deny` rule of equal or greater specificity** — this must be
   preserved exactly, it's the core security invariant.)
2. Rules producing `allow` (subject to `forceAsk` override from a hook).
3. `request.preapproved` (hook-granted for this exact call).
4. Approval dial (`auto`/`edits`) loosening `ask` → `allow`, or the doom-loop
   guard turning repeated identical asks into `deny`.
5. `PermissionRequest` hook (`beforeAsk`) — can allow or deny.
6. Unattended-turn refusal.
7. No-notify-channel refusal.
8. Ask a human; `"always"` replies write a new `allow` rule for next time.

---

## 2. Hooks (`src/main/hooks/index.cjs`, 722 lines)

Deliberately Claude Code-compatible: "a `hooks.json` written for either will
run here unchanged." Port intent: preserve the exact JSON contract so
existing user hook scripts keep working.

### 2.1 Lifecycle events (`EVENTS`, line 44)

In turn order: `SessionStart`, `UserPromptSubmit`, `PreToolUse`,
`PermissionRequest`, `PostToolUse`, `Notification`, `PreCompact`,
`PostCompact`, `SubagentStart`, `SubagentStop`, `Stop`, `StopFailure`.

**Matcher semantics per event** (`MATCHED_BY`, line 60) — what the `matcher`
regex is tested against:
| Event | matched against |
|---|---|
| PreToolUse, PermissionRequest, PostToolUse | `tool` (tool name) |
| SessionStart | `source` |
| PreCompact, PostCompact | `trigger` |
| SubagentStart, SubagentStop | `agent` |
| Notification | `kind` |
| (all others) | not matched — a matcher is nonsensical and produces a warning |

**Blockable events** (`BLOCKABLE`, line 73): only
`UserPromptSubmit, PreToolUse, PostToolUse, Stop, SubagentStop` can actually
veto/block. A `block` decision returned for a non-blockable event is
downgraded to a plain `message` (not a veto) — see `fire()`, lines 575-580.

### 2.2 Configuration sources, merge order

Three sources, later ones **added to** (not replacing) earlier ones — all
enabled handlers from all sources run for a matching event (`load()`, lines
182-216):
1. **Workspace**: `hooks/hooks.json` via
   `collections.readDocument(root, "hooks", null)` — `DOCUMENT = "hooks"`.
2. **Project**: `<cwd>/.inertia/hooks.json` — `PROJECT_FILE`, read directly
   with `fs.readFile` + `JSON.parse` (`readJsonFile`).
3. **Agent**: `agent.hooks` object on the agent record itself, source-tagged
   `agent:${agent.name ?? agent.id ?? "agent"}`.

**Read fresh every turn**, not cached — "a person editing a hook wants the
next tool call to use it, and a turn costs seconds."

**Config shapes accepted** (`parseConfig`, lines 129-163): tolerant of both
the "settings" shape (events as top-level keys) and the "plugin" shape
(`{hooks: {...}}`). Top-level keys `enabled`, `hooks`, `description`, `$schema`
are recognized meta-fields and skipped; anything else that isn't a known
event name produces a warning (not a hard error — malformed hooks degrade
gracefully with visible warnings rather than crashing the load). Each event
value must be an array of **groups**: `{ matcher?, hooks: [...], enabled? }`
(or a bare handler object at group level as shorthand). `enabled: false` at
file level, group level, or handler level all suppress that scope
(`enabled` is honored at every nesting level, per the file's stated design
goal).

**Handler normalization** (`normaliseHandler`, lines 99-122):
```js
{
  id: `${source}:${event}:${groupIndex}:${index}`,
  event, matcher: matcher ?? "",
  type: "command" | "prompt",
  command, args,      // command type: exec-form if args[] present, else shell string
  prompt,             // prompt type
  timeoutMs: (clamped 1..MAX_TIMEOUT_S=600, default 60s command / 30s prompt) * 1000,
  enabled: raw.enabled !== false,
  name, source,
}
```
A handler with neither a non-empty `command` nor a non-empty `prompt` is
dropped with a warning, never silently ignored.

### 2.3 Matching (`matches`/`select`, lines 229-246)

`matches(handler, event, subject)`:
- Event name must match exactly.
- Empty matcher or `"*"` → matches everything.
- If the event has no `MATCHED_BY` entry, matches everything regardless of
  matcher text.
- Otherwise: `new RegExp(`^(?:${pattern})$`).test(value)` — **anchored
  full-string regex match**, not substring (`shell` does NOT match a tool
  named `shell.extra`; write `shell.*` for that). Falls back to literal
  `|`-split membership test if the regex fails to compile (`pattern.split("|")`).

### 2.4 Execution model

Handlers for one event run **in parallel** via `Promise.all` (`fire()`, line
559) and cannot see each other's output. Two handler kinds:

**Command handler** (`runCommand`, lines 348-422):
- Exec-form (`command` + `args[]`) → `spawn(handler.command, handler.args, ...)`
  directly (no shell parsing needed, safe with spaces in paths).
- String-form → `spawn(shell.program, [...shell.args, handler.command], ...)`
  using the *same shell* the shell tool uses (`require("../tools/builtin/shell.cjs").SHELL`) —
  so a hook written for the user's terminal behaves identically here.
- `windowsHide: true`, stdio `["pipe","pipe","pipe"]`.
- Input: `child.stdin.end(JSON.stringify(input))` — the full event payload is
  piped as JSON on stdin.
- Timeout: `setTimeout(handler.timeoutMs)` → kill. On Windows, kill via
  `taskkill /pid <pid> /T /F` (kills the tree); elsewhere `child.kill("SIGKILL")`.
- Abort: `signal` abort → kill + resolve `{status:"cancelled", error:"The turn was stopped."}`.
- Output capped to `MAX_OUTPUT_CHARS = 20000` per stream via `clip()`.
- Exit code semantics: `0` or `2` → `status:"ok"` (2 is the *blocking* exit,
  still considered a successful, meaningful run — see `parseOutput`); anything
  else → `status:"failed"`, `error: "Exited with code ${code}."`.

**Prompt handler** (`runPrompt`, lines 434-480): puts the hook's instruction
to the model **already running the turn** (its provider/model, no tools),
requesting exactly one JSON object:
```
{"decision":"approve"|"block","reason":"<why, 1-2 sentences>"}
```
Template variable substitution in the prompt text before sending:
`$TOOL_INPUT`, `$TOOL_RESULT`/`$TOOL_RESPONSE`, `$USER_PROMPT`,
`$TRANSCRIPT_PATH`. The full event JSON (capped to 40,000 chars) is appended
after the substituted instruction. `ask` is injected by the caller
(`{ask, signal}`); if not provided, `status:"failed"` immediately ("No model
is available to evaluate a prompt hook here"). Anything the model returns
that isn't the expected JSON is read as **approval** by design — "a hook that
blocks the agent every time the model is chatty is a hook nobody keeps."

### 2.5 Output contract (`parseOutput`, lines 271-332)

Reads Claude Code's hook output format in every spelling it has had:

- **Exit code 2** → always blocking: `out.block = true`, `out.reason` from
  stderr (fallback stdout, fallback `"Blocked by a hook."`). For
  `PreToolUse`/`PermissionRequest` also sets `out.decision = "deny"`.
- **Non-JSON stdout** (plain text) → routed to `context` (fed back to the
  model) for `UserPromptSubmit`, `SessionStart`, `PostToolUse`,
  `SubagentStart`; otherwise treated as a `message` shown to the person.
- **JSON stdout**, fields recognized:
  - `continue: false` → `out.stop = true`, `out.stopReason`.
  - `systemMessage: string` → `out.message`.
  - `decision: "block"|"deny"` → `out.block = true`, reason from
    `reason` or `hookSpecificOutput.permissionDecisionReason`.
  - `hookSpecificOutput.permissionDecision` (or top-level `decision:"approve"|"allow"`)
    → `out.decision ∈ {allow, deny, ask}`; `deny` also sets `block`.
  - `hookSpecificOutput.decision.behavior` (`PermissionRequest`'s own spelling:
    `{decision:{behavior:"allow"|"deny", message}}`) → same semantics, message
    from `decision.message`.
  - `hookSpecificOutput.updatedInput` → `out.updatedInput` (PreToolUse only —
    **a hook can rewrite the tool's arguments before execution**).
  - `hookSpecificOutput.additionalContext` or top-level `additionalContext`
    (string) → pushed onto `out.context`.

**Merging results across parallel handlers** (`fire()`, lines 629-647):
- First `block` wins (`!merged.block` guard — first blocker's reason sticks).
- `decision` uses a rank table `{deny:3, ask:2, allow:1}` — **most cautious
  decision across all handlers wins**, regardless of order.
- `updatedInput` — first non-null wins (`!merged.updatedInput` guard).
- `context` — **all** handlers' context strings are concatenated (accumulate,
  not first-wins).
- `message` — concatenated with `\n` if multiple handlers produce one.
- `stop` — sticky OR; first `stopReason` wins.

A `block` returned on a non-blockable event is downgraded: it becomes a
`message`, `block`/`reason` are cleared (lines 575-580) — so `PostCompact`
can't actually stop anything, only comment.

### 2.6 Environment variables passed to command hooks

```
INERTIA_PROJECT_DIR, INERTIA_WORKSPACE, INERTIA_SESSION_ID, INERTIA_AGENT,
INERTIA_HOOK_EVENT, CLAUDE_PROJECT_DIR (Claude Code compat alias)
```
plus the full parent `process.env`.

### 2.7 Payload sent on stdin (`fire()`, lines 534-546)

```js
{
  session_id, thread_id, turn_id, agent_name, agent_id, cwd, workspace,
  model, permission_mode /* = context.conversationMode */,
  hook_event_name: event,
  ...input,           // event-specific fields, e.g. tool_input for PreToolUse
}
```

### 2.8 Timeout and error handling

- Default timeouts: 60s command, 30s prompt (`DEFAULT_COMMAND_TIMEOUT_S`,
  `DEFAULT_PROMPT_TIMEOUT_S`); user-settable up to `MAX_TIMEOUT_S = 600`.
- A handler that **crashes, errors, or times out is never a reason to end the
  turn** — it is logged to `failures.cjs` (`kind: "hook"`) and surfaced as a
  transcript event, but the turn proceeds as if that handler had produced an
  empty outcome. Explicit design rule: "a broken hook must cost the user the
  hook, not the agent."
- Every run (success or failure) is pushed to an in-memory ring buffer
  (`RECENT`, capped at `MAX_RECENT = 100`) and broadcast to `onRun` listeners
  — this is what feeds the settings pane's live "recent hook runs" list.

### 2.9 IPC surface (`hooks/ipc.cjs`) — for reference, not required in Tauri (replace with Tauri commands)

`hooks:list`, `hooks:recent`, `hooks:write-example` (refuses to overwrite an
existing file), `hooks:set-enabled` (top-level `enabled` toggle in the
workspace file), `hooks:reveal` (opens the file/folder in the OS file
manager), plus a push channel `hooks:run` for live updates.

---

## 3. Routines (`src/main/routines/{index,schedule}.cjs`, 669 + 299 lines)

A routine is **a Markdown playbook plus a schedule**, run as an ordinary agent
turn with `unattended: true`. Explicitly lives in the main/background process
(not the renderer) because "a schedule that only fires while a chat screen is
mounted is not a schedule."

### 3.1 Schedule model (`schedule.cjs`)

Four schedule `kind`s, stored as `routine.schedule = { kind, expression, nextRunAt }`:
- **`manual`** — `nextRun` always returns `{at: null}`; never auto-fires.
- **`trigger`** — same, reserved for a future event-based trigger (not
  implemented as a clock trigger here — currently behaves like manual for
  scheduling purposes).
- **`once`** — `expression` is an ISO-8601 instant (`parseMoment`, strict
  `Date.parse`, throws a readable error on non-parseable input). Fires once;
  "already ran" is decided by comparing `lastRun.at >= parsed instant`. **A
  due time in the past still fires once** if the app was closed when it was
  due (not silently dropped).
- **`interval`** — `expression` is an ISO-8601 duration restricted to weeks/
  days/hours/minutes/seconds (`parseDuration`, regex
  `^P(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$`).
  **Minimum interval is 60,000ms (one minute)** — hard floor, throws
  otherwise. Next run is measured from `lastRun.at` if present, else from
  `now`, and is always at least 1 second in the future
  (`Math.max(base+every, now+1000)`).
- **`cron`** — standard 5-field cron (`minute hour day month weekday`), a
  **hand-written parser** (deliberately, not a dependency — comment explains
  the whole surface is 5 fields/4 operators, and a scheduler dependency has to
  be trusted with firing the user's work on time). Supports:
  - Comma lists, `a-b` ranges, `/step`, named weekdays (`sun..sat`) and months
    (`jan..dec`), case-insensitive, 3-letter-prefix matched (`named()`).
  - `weekday` field: both `0` and `7` mean Sunday.
  - **Day-of-month/weekday OR rule** (classic cron gotcha, replicated
    faithfully): when *both* day and weekday fields are restricted
    (non-`*`), a date matches if *either* matches (OR), not both (AND). When
    only one is restricted, that one alone must match.
  - Search: minute-by-minute forward scan from `now` (rounded up to the next
    whole minute) for up to `SEARCH_LIMIT_MINUTES = 366*24*60` (~1 year)
    before giving up (`"That schedule never comes round."`).
  - **Local time, deliberately** — matched against the host's local calendar
    fields, not UTC, explicitly to get "9am" right for the user's timezone
    (accepting DST skip/repeat edge cases, same as real cron).

**`isDue(routine, now)`** (line 247): `false` if disabled, `manual`,
`trigger`, or already `lastRun.status === "running"` (serialization guard —
"two copies of one routine racing each other is worse than a late run").
Otherwise compares `Date.parse(routine.schedule.nextRunAt) <= now`. **No
recorded `nextRunAt` → not due** (the scheduler computes and stores it on the
same tick before checking due-ness).

**`describe(routine)`** — human-readable summary string for UI, no clock
access needed.

### 3.2 Execution model (`index.cjs`)

**Ticking**: `setInterval` every `TICK_MS = 30_000` (30s — finer than a cron
minute would need, but cheap), `timer.unref()` so it never blocks process
exit. One tick on startup as well (`tick().catch(()=>{})` before the interval
starts) so routines due while the app was closed fire shortly after launch.

**Serialization**: a module-level `running` boolean makes `tick()` a no-op
(`{ran:[], skipped:"busy"}`) if a previous tick's batch hasn't finished — **all
due routines in one tick run sequentially** (`for (const routine of due) { ...
await runOnce(...) }`), not concurrently. This is deliberate: "an agent's
working folder is not a place two turns should be editing at once."

**In-flight tracking**: a `Set<routineId>` (`inFlight`) is the *authoritative*
source for "is this routine actually running in this process" — distinct from
the on-disk `lastRun.status === "running"`, which can lie if a previous
process crashed mid-run. `settleStale()` (called once, lazily, on the first
tick) rewrites any on-disk `"running"` record not backed by an in-flight entry
to `status: "warning", summary: "Interrupted: the app closed while this was running."`

**Paused agents get no work**: `pausedAgents(root)` reads the agents
collection fresh every tick (`isPaused(agent)` from `shared/agents.js`) and
filters `due` routines whose `agentId` is paused. A routine's `nextRunAt` plan
is *not* advanced while paused — it just waits, so a pause postpones rather
than skips a run.

**`runOnce(root, routine, {manual, timeoutMs = RUN_TIMEOUT_MS = 15min})`**
(lines 138-252) — the actual run:
1. Writes `lastRun: {at, status:"running", durationMs:0, summary}` immediately.
2. Opens a transcript: a dedicated conversation thread `routine-<id>`
   (`threadIdFor`), reusing/creating the thread record and appending a
   `user` message (the playbook text, see below) and a `streaming` `agent`
   reply placeholder — using the exact same message/thread shape as an
   ordinary chat conversation, so the chat UI renders it with no special case.
3. Calls the injected `startTurn(config, sendTurnEvent)` with:
   ```js
   {
     threadId: transcript.threadId,
     messageId: transcript.messageId,
     agentId: routine.agentId,
     history: [{ role: "user", content: playbook(routine) }],
     unattended: true,
     conversationMode: routineMode(routine),    // from shared/routines.js
     approval: routineApproval(routine),         // from shared/routines.js
   }
   ```
4. Polls (`waitForTurn`, `POLL_MS = 1000` default, test-overridable) the
   injected `readTurn(turnId)` until `status !== "running"` or `timeoutMs`
   elapses. On timeout, calls injected `cancelTurn(turnId)` and records
   `status:"warning"`.
5. Folds the turn's recorded events into the transcript's reply message via
   `foldRecorded` (`shared/transcript.js`) and writes it back —
   **idempotently**: a window that was watching live already wrote the same
   fold from the same events, so writing it again here (for the unattended
   case) is harmless.
6. On any thrown error (turn couldn't start, workspace gone, etc.) — records
   to `failures.cjs` with `kind: "routine"` and marks the run `status:"error"`.
7. Always (`finally`): removes from `inFlight`.
8. Writes the final `lastRun`, prepends to `runHistory` (capped at
   `MAX_HISTORY = 20`), and recomputes `schedule.nextRunAt` via
   `schedule.nextRun({...routine, lastRun: run})`.
9. Broadcasts `routine:started` / `routine:finished` events.

**The playbook message** (`playbook(routine)`, lines 369-386) — sent as the
`user` turn content, always prefixes context telling the model it is running
unattended and must not ask questions, plus **approval-mode-specific
guidance** baked into the prompt text itself (auto / edits / ask — see §1.4)
so the model plans around what will and won't be refused, followed by the
routine's raw markdown.

**Manual run** (`runNow(routineId)`): same `runOnce` with `manual: true`.
Refuses if already `inFlight`, or if the owning agent `isPaused` (checked
*before* writing any run record, to avoid polluting history with a refusal
that duplicates the pause banner).

### 3.3 Config shape (routine record, as read/written)

```js
{
  id, name, agentId, markdown | description, enabled: bool,
  schedule: { kind: "manual"|"trigger"|"once"|"interval"|"cron", expression, nextRunAt },
  lastRun: { at, status: "running"|"success"|"error"|"warning", durationMs, summary, threadId },
  runHistory: [ /* up to MAX_HISTORY=20, newest first */ ],
}
```

### 3.4 Result handling / status vocabulary

`status ∈ {"success", "error", "warning"}` (never a bare "failed"/"ok" pair —
`"warning"` specifically covers cancellation/timeout/interruption, which the
code explicitly does *not* want conflated with a red "error" row: "a
cancelled turn is not a failure and not a success... calling it an error
would put a red row in the history for an orderly shutdown.") `summary` is
derived from the turn's last paragraph of assistant text (`summarise()`,
lines 441-454), truncated to 280 chars, or the first recorded error event's
message if there was no text at all.

### 3.5 Rust design notes for routines

- Scheduler tick is a natural fit for a Tokio `interval` task; the "one tick
  running at a time" and "routines run sequentially within a tick" invariants
  should be enforced with an `AtomicBool`/`Mutex` guard exactly like `running`
  here — don't parallelize the `for routine in due` loop even though it's
  tempting.
- `startTurn`/`cancelTurn`/`readTurn`/`broadcast`/`sendTurnEvent` are all
  **injected function pointers** in the JS version specifically to keep the
  scheduler decoupled and testable without an Electron window. Model this as
  a trait, e.g. `trait TurnRunner { fn start(&self, cfg: TurnConfig) -> TurnHandle; fn cancel(&self, id: TurnId); fn read(&self, id: TurnId) -> Option<TurnRecord>; }`.

---

## 4. Sandbox (`src/main/sandbox/`)

### 4.1 Provider trait (`provider.cjs`, contract documented at lines 16-46)

Every provider (`local.cjs`, `docker.cjs`, `daytona.cjs`) exports the same
function set — this is the seam that makes a fourth provider "a file plus a
line in the registry":

```
id, label, blurb
available(config) -> { ok, reason, hint? }        // never bare false — always a reason
create(spec)       -> { handle, name?, status, os, image, workdir, specs }
start / stop / remove (handle, ctx)
status(handle, ctx) -> { status, startedAt? }       // status ∈ STATUSES
stats(handle, ctx)  -> { cpuPct, memPct, diskPct, scope }  // nulls for "can't measure", never fake 0
exec(handle, request) -> { code, stdout, stderr, durationMs, timedOut, ok }
listDir(handle, path, ctx) -> Entry[]
readFile / readFileBase64 / writeFile
snapshot / snapshots / restore
screenshot(handle, ctx) -> data URL | null
screen(handle, ctx) -> { view, control } | null      // optional; only docker/daytona implement
```

**`STATUSES = ["provisioning","running","paused","stopped","error","missing"]`**
(line 50).

**Two invariants every provider must uphold** (explicitly documented, and
must be preserved in the Rust port):
1. **Nothing throws for a machine that simply isn't there.** `status()`
   answers `"missing"`; other calls degrade the same way a stopped machine
   would. This must render as "gone," not an error dialog.
2. **A command's exit code is not an error.** `exec()` *resolves* (not
   rejects) for a non-zero exit; it only rejects if the command genuinely
   could not be started/reached. The agent must be able to distinguish "the
   test suite failed" (needs a different reaction) from "the machine is
   unreachable."

**Shared helpers** in `provider.cjs`: `execResult()` normalizer,
`entry()` directory-row normalizer, `stats()` percent clamp+round(1 decimal)+
null-passthrough, `toShellLine`/`quoteArg` (argv→shell-string with POSIX
single-quote escaping), `machineName(id)` (slugifies to `inertia-<slug>`,
`NAME_PREFIX = "inertia-"`), and a shared `SCREENSHOT_COMMAND` +
`screenshotVia(exec)` helper used identically by docker and daytona: runs
`import -display $DISPLAY -window root png:- | base64 -w0` inside the
machine, validates the result by checking the base64 output starts with the
literal magic string `"iVBORw0K"` (PNG header, base64-encoded) rather than a
size heuristic (a documented bug-fix: a flat-colored screen can compress
under any reasonable size threshold and false-positive as "no display").

### 4.2 Registry & orchestration (`index.cjs`)

`PROVIDERS = {local, docker, daytona}`, `PROVIDER_ORDER = ["local","docker","daytona"]`
(display order, least setup first). All higher-level app code (IPC, panes,
the agent's `computer` tool) calls only the functions exported from
`index.cjs`, never a provider directly.

**Settings** (`SETTINGS_DOC = "settings.computers"`, workspace document):
```js
{
  defaultProvider: "docker",
  daytonaApiUrl: "",
  daytonaImage: "",          // registry ref or snapshot name Daytona should pull
  autoStopMinutes: 30,        // Daytona bills by the minute; idle machines stop themselves
  defaultSpecs: { cpu: 2, memoryGb: 4, diskGb: 20 },
  pollSeconds: 10,
}
```
`configFor(root, providerId)` reads Daytona's API key/URL from the **secrets
store** fresh on every call (never cached, so a key pasted mid-session takes
effect without a relaunch).

**A machine is a workspace-collection record** (`computers` collection) — the
same "a file you can read, diff and delete" pattern as every other resource
in the app. The one field that makes it a live machine rather than a note is
`handle` (opaque string the provider owns the meaning of: container id,
Daytona sandbox id, or local directory path).

**Create flow** (`create()`, lines 210-282): writes the record **twice** —
once as `status:"provisioning", handle:null` *before* calling the provider
(so a crash mid-provision never leaves an orphan container the app can't
see/stop/delete), then patches in the real `handle`/`status`/`os`/`workdir`/
`specs` once the provider answers. On provider error, the record is patched
to `status:"error", error: message` and kept (not deleted) — "a row that says
why it failed is something the user can retry or delete; a row that vanished
is a mystery."

**`status` is asked of the provider, never trusted from the stored record**
(`refresh()`) — a container the user removed via `docker rm` by hand, or a
Daytona sandbox reaped for idleness, must show as `missing` rather than
`running` forever.

**Lifecycle dispatch** (`lifecycle(root, id, verb, opts)`): generic — looks up
`provider[verb]` as a function and calls it; throws if the provider doesn't
support that verb (e.g. `local` has no `pause`/`unpause`).

**One machine per agent**: `assign()` (lines 568-588) enforces this — setting
an agent's `computerId` clears any other machine it was previously assigned
to. `reconcile()` (lines 603-635), run once at startup, repairs dangling
references both directions (agent→deleted-computer, computer→deleted-agent).

**`hasDesktop(machine)`** (line 664): whether a machine has a screen/browser
— true only for Docker (always, since it always runs the app's own image) or
a Daytona machine whose recorded `image` name contains the sandbox image's
`name` from the manifest. `local` is always `false`.

**`workdirFor(machine)`** (line 658): `machine.workdir` if recorded, else
`/home/daytona` for daytona, else `/workspace` (Docker default fallback for
pre-existing records made before `workdir` was tracked).

**`drive(root, id, action, args)`** (lines 490-525): the single entry point
for both the Browser pane *and* the agent's `computer` tool — explicitly
unified because they used to diverge (different launch flags, profile dirs,
pid files) and a fix to one stopped fixing the other. Actions:
`open, close, pageText, windows, observe, click, type, key, scroll` — each
compiles to a `desktop.*` command batch (see §4.4) and is run via `exec()`.
Exit code `3` is the agreed-upon "this machine cannot do that" sentinel from
`desktop.cjs` (surfaced as `{unsupported: true}`, not thrown, so the UI can
render the reason rather than a toast).

### 4.3 Docker provider (`providers/docker.cjs`)

Driven via the `docker` **CLI binary** (`spawn("docker", args)`), deliberately
not the Engine API/socket — cross-platform (no Windows named-pipe client
dependency), and CLI errors are the same errors a user would get typing the
command themselves (pasteable into search). One process spawn per operation.

- `available()`: `docker version --format {{.Server.Version}}`, 8s timeout
  (`PROBE_TIMEOUT_MS`). Distinguishes "binary not on PATH" (spawn failure)
  from "daemon not running" (exit code nonzero + stderr matches
  `cannot connect|daemon|pipe`) — different fixes, different messages.
- `ensureImage()`: `docker image inspect <ref>`; if missing, builds from
  `sandbox/Dockerfile` (`docker build -t <ref> -f ... <dir>`, 15-minute
  timeout `BUILD_TIMEOUT_MS`). Manifest-driven (`sandbox/image.cjs`).
- `create()`: `docker run -d --name inertia-<slug> --label inertia=1 -w <workdir>
  [--cpus N] [--memory Ng] -v <name>-workspace:<workdir>
  -p 127.0.0.1::6080 -p 127.0.0.1::6081 <image>`.
  - Named volume (not bind mount) so the container's disk survives `docker rm`
    and the container can't reach the host filesystem via an arbitrary host
    path.
  - Screen ports published to **loopback only** (`127.0.0.1::PORT`, no fixed
    host port — Docker assigns one so multiple machines don't collide), never
    to `0.0.0.0` — explicit invariant: "a container with a browser someone is
    signed into does not belong on the LAN."
  - Removes any stale container of the same name first (`docker rm -f`) to
    avoid a name-collision failure reading as an app bug.
- `exec()`: `docker exec [-w cwd] [-e K=V]... <handle> <manifest.shell...> <line>`.
  Timeout enforced client-side (kill on timer).
- `listDir()`: `find -mindepth 1 -maxdepth 1 -printf '%y\t%s\t%T@\t%f\n'`
  (explicitly not `ls -l`, whose output is locale-dependent and ambiguous
  with filenames containing spaces), capped to 200,000 bytes via `head -c`.
- `readFile`/`readFileBase64`: `cat`/`base64 -w0` over exec.
- `writeFile`: pipes base64-encoded content into
  `mkdir -p $(dirname X) && base64 -d > X` via stdin — never `echo`, to avoid
  every shell-quoting/heredoc/non-UTF8 hazard of embedding arbitrary content
  in a command line.
- `snapshot`/`snapshots`/`restore`: `docker commit` to a tag
  `<name>-snapshots:snap-<ts36>`; restoring **replaces the container**
  (removes old handle, runs a new one from the tag) — there's no in-place
  filesystem rollback for a running container, so the handle changes and the
  caller must persist the new one.
- `screen()`: reads the two published host ports back via `docker port`
  (never cached — `docker start` reassigns fresh ports every time).
- `stats()`: `docker stats --no-stream --format {{.CPUPerc}}|{{.MemPerc}}|{{.MemUsage}}`;
  disk is always `null` (no quota on a Docker volume to be a percentage of).

### 4.4 Daytona provider (`providers/daytona.cjs`)

HTTP-based (`fetch`), cloud sandboxes that outlive the app process. Two
distinct API surfaces:
- **Control plane** (`ROUTES`, api host, Bearer auth from
  `secrets.get(root, "DAYTONA_API_KEY")`): create/get/list/start/stop/remove/
  snapshot/snapshots/preview.
- **Toolbox** (exec, file listing, download): lives on a *separate proxy*
  whose base URL is read from the sandbox object's `toolboxProxyUrl` field
  and cached per-handle in a `Map` (`proxies`) — cleared on `remove()`.

Key behaviors that must be preserved (found empirically against the live
service, per code comments — high value for a Rust port to get right the
first time rather than rediscover):
- **Never send resource specs on create** — Daytona rejects `cpu`/`memory`/
  `disk` alongside any image or snapshot ("every sandbox is made from one").
  Size is entirely determined by which snapshot is chosen.
- `image` (a registry ref, has `:` or `/`) vs `snapshot` (bare name) are
  distinguished by string shape (`looksLikeRegistryRef`) and sent as
  different body fields.
- If our own default snapshot name is rejected as "not found" (because it
  isn't registered in that Daytona org) **and** we supplied that name as a
  default (`imageIsDefault`), silently retry with neither field, falling back
  to Daytona's own base image. If the user explicitly typed a name that 404s,
  surface a real error instead.
- `create()` polls (`waitForReady`, exponential backoff 500ms→5s cap, 180s
  overall deadline) until state is `running`.
- **`cwd` must never be sent to the exec toolbox call** — Daytona's daemon
  routes a request carrying `cwd` through the sandbox user's login shell,
  which the image sets to zsh but doesn't install, producing a cryptic
  `fork/exec /usr/bin/zsh: no such file or directory`. Instead, `cwd` is
  folded into the command as a `cd <dir> && <cmd>` shell prefix.
- `ensureDesktop()`: Daytona overrides the image's own start command, so a
  sandbox made from the app's own snapshot boots with no X server running
  even though `xdotool`/`import` are present. After create/start, this runs a
  detection+bootstrap script over exec (`setsid nohup /usr/local/bin/inertia-computer &`,
  detached with `</dev/null` so the exec daemon reaping the process tree on
  return doesn't kill it) that's a no-op if the binary is absent (Daytona's
  own base image) or the display is already up.
- `readSandbox()`/`readExec()` are the **only** two functions that touch
  Daytona's vendor vocabulary (state-name mapping, exec response field
  names) — isolating "the API might rename a field" to two functions and a
  `ROUTES` table, per the file's own design note.
- **Preview/screen URLs**: `GET /sandbox/{id}/ports/{port}/preview-url` per
  port (6080 view / 6081 control), returns a public hostname + token. Token
  goes in the query string for the initial page load (an iframe can't set a
  custom header), and is remembered in a `previewTokens: Map<host, token>`
  keyed by hostname so a later HTTP proxy layer can attach it as the header
  `x-daytona-preview-token` (`PREVIEW_HEADER`) on subsequent requests the
  browser makes for scripts/styles (query-param auth doesn't propagate to
  those). **Both view and control URLs must resolve or neither is returned**
  ("half a screen is not worth showing").
- 404 from the **control plane** is read as `null` (ordinary "gone" state);
  404 from the **toolbox** is a real error (wrong route / daemon down) — this
  distinction (`request()` vs `toolbox()`, two separate HTTP helper
  functions) must be preserved, since collapsing them makes a broken exec
  look like a command that printed nothing.

### 4.5 Local provider (`providers/local.cjs`)

Runs directly on the host — **explicitly not a sandbox** ("a command run
here has the user's permissions and can reach anything the user can" —
`available()` returns this as a `warning` string surfaced in the UI, not a
hidden caveat). Exists mainly to (a) work with zero external dependencies for
offline/no-Docker use, and (b) prove the provider interface with results a
user can inspect directly in their file manager.

- **Containment that does exist**: every path is resolved through
  `resolveInside(home, target)` (lines 61-70), which strips a leading
  `/workspace/` prefix (so agent-authored paths using the virtual mount point
  still work) then does `path.resolve` + `path.relative` and **refuses**
  (`throw`) if the relative path starts with `..` or is still absolute — this
  is the entire security boundary for this provider and must be ported
  exactly (a symlink escape inside `home` is *not* defended against, matching
  current behavior — worth flagging but not silently "fixing" beyond parity
  unless asked).
- Machine "home" directory: `<workspaceRoot>/files/machines/<machineId>`.
  Snapshots: `<workspaceRoot>/files/machines/<machineId>.snapshots/` — plain
  recursive directory copies (`fs.cp`) plus a JSON sidecar manifest per
  snapshot, not a filesystem-level snapshot (no privileges required, fully
  portable, "slower and completely honest").
- `exec()`: spawns the host's own shell (`cmd.exe /d /s /c` on Windows,
  `$SHELL -lc` else) with `cwd` resolved inside `home`.
- `stats()`: host-wide, not per-machine — CPU sampled by differencing
  `os.cpus()` idle/total between two calls (first call has no baseline, is
  always `null`), memory from `os.totalmem/freemem`. Explicitly labeled
  `scope: "host"` since there's no cgroup to scope it to a single machine
  (this app process's own CPU/memory is folded in).
- `screenshot()`: **always returns `null`** — deliberately, since the "screen"
  would just be the user's own live desktop, not something meaningfully
  captured for the pane.

### 4.6 Driving the desktop (`sandbox/desktop.cjs`)

Actions are **data, compiled to shell/xdotool commands at the last moment**,
batched into one `exec()` round-trip (up to `MAX_ACTIONS = 24` per batch) —
explicitly to avoid the cost of one tool-call round-trip (with a full
screenshot each time) per click/type/keypress. Kinds:
`click, move, down, up, type, key, scroll, wait`.
- Every generated command is prefixed with `export DISPLAY="${DISPLAY:-:99}"`
  and a guard (`NEEDS_DISPLAY`) that exits with code **3** and a descriptive
  message if `xdotool` is absent or the display can't be reached — this is
  the sentinel `drive()` in `index.cjs` reads as `unsupported: true`.
- `pkill -f` is explicitly banned in these commands (a documented footgun: a
  pattern matching a browser process also matches the shell that launched it
  via its own argv, so it self-kills before launching).
- "Typing is a paste" (clipboard-based, not raw keystrokes) — reduces the
  attack/fragility surface of typing arbitrary model-authored text through a
  shell-interpolated `xdotool type`.
- Coordinates and durations are all bounds-checked (`coordinate()`,
  `bounded()`) before being interpolated into a shell command —
  `MAX_SETTLE_MS = 5000`, `MAX_WAIT_MS = 30000`.
- Very tolerant input parsing (`liftActions`) — the model reliably sends
  malformed shapes (bare string, top-level fields instead of nested, etc.)
  and the parser normalizes rather than rejecting, because "refusing it cost
  the whole task" in practice.

### 4.7 Image manifest (`sandbox/image.cjs`)

`sandbox/image.json`, resolved relative to the source tree (dev) or
`process.resourcesPath` (packaged build) or cwd (fallback), cached after
first read (requires app restart to pick up an edit). Fields:
`name, tag, dockerfile, workdir=/workspace, user=agent, shell=[/bin/bash,-lc],
daytonaFallbackImage=debian:12-slim, base=Debian 12, registry, daytonaSnapshot`.

---

## 5. Support modules

### 5.1 `background.cjs` — window-close / tray behavior

Two settings, stored in **completely different places** (worth preserving
this split, not collapsing it):
- `minimiseToTray` (default `true`) — a workspace preference, stored in the
  `settings.app` workspace document (`DOCUMENT = "settings.app"`), so it
  travels with the workspace folder like every other user-visible setting.
  Cached in memory (`cached`) because a window's synchronous `close` event
  handler cannot `await` a disk read — `load()` populates the cache at
  startup, `save()` refreshes it on every write, `current()` is the
  always-synchronous read the close handler uses.
- `launchAtLogin` — **not stored anywhere by this app**; read live from the
  OS (`app.getLoginItemSettings()` in Electron) every time it's displayed, so
  the toggle reflects what the OS actually does (the user can undo it in Task
  Manager/Login Items without the app knowing, and a cached value would lie).
- `trayTooltip(name, runningTurns)`: pure string formatter —
  `"${name} - running in the background"` / `"1 turn running"` /
  `"${n} turns running"`.

**Rust/Tauri port**: `minimiseToTray` → a workspace-scoped config value read
synchronously (or cached at startup and refreshed on save) so the window
close handler can decide without blocking; `launchAtLogin` → query the OS
autostart API live each time (Tauri has `tauri-plugin-autostart` or native
equivalents) rather than mirroring state.

### 5.2 `notify.cjs` — user-facing notifications

Two channels, chosen by whether the window is **focused** (not merely
visible — `watching(win)` checks `isFocused()`, not `isVisible()`):
- Focused → in-app toast only, sent via `sendToWindow(win, "notify:event", {...})`.
- Unfocused → OS-level `Notification` (Electron's), clicking it restores/
  shows/focuses the window. Falls back to "toast" reporting if
  `Notification.isSupported()` is false or construction throws (headless/
  no notification daemon).
- **Both cases always send the `notify:event` IPC message** regardless of
  focus — the renderer decides whether to render a visible toast, but the
  event is always recorded so a reopened window's notification list is
  complete.
- `tone ∈ {"info", "problem"}` — only affects toast styling; OS banners have
  no equivalent dial on any supported platform (`silent: tone !== "problem"`
  is the one OS-level effect: a "problem" banner is not silent).
- Explicit policy on **what's worth a banner** (should be preserved as
  product behavior, not just code): only "a run failed" and "a team
  finished" warrant an OS notification. Not: run starting, run finishing
  while siblings still run, or cancellation (a user-initiated action doesn't
  need to be echoed back as a notification).

### 5.3 `failures.cjs` — the structured failure log (agent-queryable)

Distinct from `log.cjs` (developer text log) — this is **structured JSON,
meant to be read back by an agent** ("why does the build keep failing"), not
a human pasting into a bug report. Location: `logs/failures/` under the
workspace.

- **Retention**: 90 days (`KEEP_DAYS`), one JSON-lines file per day
  (`YYYY-MM-DD.jsonl`), append-only (crash-safe — a partial write mid-crash
  corrupts at most the last line, not the file). Per-day file capped at
  `MAX_BYTES = 8MB`; once hit, that day simply stops recording (protects
  against a crash loop filling the disk) rather than rotating.
- **Redaction is mandatory and layered** (`redact()`, lines 98-118) — **this
  must not be lost in the port**, it's explicitly the security-relevant
  contract for this module:
  1. **Exact-match pass**: every known secret value (from the workspace's own
     secrets store, refreshed via `useSecrets(values)`, min length 6) is
     string-replaced with `[REDACTED]` wherever it appears verbatim in the
     text.
  2. **Shape-based regex passes**, applied in this order (`PATTERNS`, lines
     88-96):
     - `sk-`/`rk-`/`pk-` + 16+ alnum/`_`/`-` (OpenAI/Stripe-style keys)
     - `ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_` + 20+ chars (GitHub tokens)
     - `glpat-`/`gldt-`/`glrt-`/`gloas-`/`glptt-`/`glagent-`/`glimt-`/
       `glsoat-`/`glcbt-`/`glft-`/`glffct-` + 16+ chars (GitLab tokens)
     - `xox[abprs]-` + 10+ chars (Slack tokens)
     - `AKIA[0-9A-Z]{16}` (AWS access key ids)
     - `Bearer <16+ chars>` (any bearer token) → replaced as
       `"Bearer [REDACTED]"` (keeps the word "Bearer" visible, only the
       value is hidden)
     - Generic key=value shape: `(api[_-]?key|token|secret|password|passwd
       |authorization|access[_-]?key)(["']?\s*[:=]\s*["']?)([^"'\s,;}&]{6,})`
       — case-insensitive, value ≥6 chars, replaces only the value
       (skips re-redacting a value that's already literally `bearer` or
       `[redacted]`, to avoid double-mangling text the Bearer pattern
       already touched).
  - Applied to **every free-text field** stored: `error`, `args`, `output`,
    `stack`, `prompt` — none of these fields may bypass redaction.
- **Per-field caps** (truncate, don't drop): `args`/`output` 4000 chars,
  `error` 2000, `stack` 3000, `prompt` 1000 (`CAPS`). `output` is capped from
  the **tail** (`tail()`, not `cap()`/head) — "a failed build's error is at
  the end."
- **Signature for deduplication** (`signatureOf`, lines 162-174): normalizes
  the first line of the error by replacing Windows/POSIX paths, quoted
  strings, hex literals, and numbers with placeholders
  (`<path>`, `<str>`, `<hex>`, `<n>`), lowercased, capped to 160 chars,
  composed as `` `${kind}|${tool}|${normalizedError}` ``. This is what lets
  "the same failure happened again" collapse into one queryable group instead
  of N visually-different lines.
- **`KINDS`** (line 51): `tool, tool-arguments, tool-unknown, permission,
  provider, turn, turn-limit, loop, routine, subagent, mcp, lsp, hook, crash,
  other` — anything else on input coerces to `"other"`.
- **Never throws** (`record()` wraps everything in try/catch, returns
  `null`/partial on failure) — every caller is itself inside an error path.
- In-memory ring buffer (`RING_SIZE = 500`) always populated even if the
  directory can't be created, so the query/summary API still answers within
  the current process session.
- `query()`/`summary()`: text search across error+args+output+tool,
  filterable by kind/tool/agent/days, with per-row `seen` counts from the
  signature grouping and a top-10 "most repeated signatures" summary.

**Rust port note**: redaction regexes must be ported byte-for-byte (or with
equivalent coverage validated against the same test fixtures in
`failures.test.js`, not read here but implied by the module structure) —
this is the one piece of this whole doc explicitly flagged in the task
prompt as "must not be lost."

### 5.4 `security.cjs` — Electron webview/navigation hardening

**Not a secrets module** — despite the filename's proximity to
"security-relevant," this file is entirely about **what the Electron
`BrowserWindow` is allowed to do**: permission grants and navigation/URL
gating. In a Tauri port, the *concepts* map onto Tauri's own CSP/allowlist
and IPC scoping config rather than needing bespoke code, but the **policy
decisions** below must be replicated in whatever mechanism Tauri uses:

- **Permission allowlist** (`ALLOWED_PERMISSIONS`): only `media` (mic only,
  never camera — `permissionDecision` explicitly excludes any request whose
  `mediaTypes`/`mediaType` includes `"video"`) and
  `clipboard-sanitized-write` (write-only; `clipboard-read` is never
  granted). Every other Chromium permission (geolocation, notifications,
  MIDI, HID, serial, etc.) is denied by default — inverted from Chromium's
  own defaults, justified as "no caller in this app has a legitimate use for
  these."
- **Origin check for permission grants** (`isAppUrl`): a permission is only
  ever granted to a request whose origin is the app's own page — dev-mode
  Vite origin (`http://localhost:5173` exact match, or that origin as a
  prefix) or, in production, the **exact built `index.html` file** (not the
  whole `dist/` directory — explicitly not just "any asset under dist,"
  since a model-authored markdown link pointing at some other file in
  `dist/` must not be treated as "the app"). An embedded remote-desktop
  iframe (a sandbox pane) must never be able to borrow the microphone or
  clipboard write — this is the concrete attack this file defends against.
- **`filePathOf(url)`**: hand-rolled `file://` URL → path decoder, explicitly
  *not* using Node's `fileURLToPath` because that throws on a UNC host or an
  invalid percent-escape — both of which a hostile URL might contain, and
  the permission-check code path must not crash (fail closed, not crash
  open) when handed one.
- **Path comparison** (`normalisePath`): case-insensitive on Windows
  (`process.platform === "win32"`), forward-slash normalized, always
  `path.resolve`d first (defeats `..` traversal and drive-letter/relative
  ambiguity in the comparison).

**Rust/Tauri equivalent**: Tauri's CSP + `tauri.conf.json` allowlist/scope
config, plus the `asset:` protocol scoping, largely replace this file's
*mechanism*. The **policy** — mic-only no-camera media grants, no clipboard
read, origin-restricted to the app's own built page exactly (not a whole
directory) — must be explicitly configured to match; it will not happen by
Tauri's defaults.

### 5.5 `log.cjs` — developer/operational text log

Plain-text, human-readable, `logs/<YYYY-MM-DD>.log` under the workspace (or
`app.getPath("userData")/logs` before a workspace is chosen — this fallback
matters because crash handlers are registered before the workspace is known).

- **Synchronous writes** (`fs.appendFileSync`) — deliberate, the one thing
  this module must do reliably is record an `uncaughtException` on the way
  out of the process, and an async write loses that race (process exits
  before the write flushes). Low volume by construction (warnings/errors
  only, not per-tool-call tracing) makes the blocking-write cost acceptable.
- Rotation: `MAX_BYTES = 2MB` per day; past that, `rotate()` finds the lowest
  free numbered slot `<day>.<n>.log` up to `MAX_ROTATIONS = 5`, or truncates
  the current file to empty if all 5 slots are taken (bounded disk use over
  correctness in a runaway-logging scenario).
- Retention: `KEEP_DAYS = 7` (much shorter than `failures.cjs`'s 90 — this is
  for "what just happened," not queryable history).
- `attachConsole()`: monkey-patches `console.warn`/`console.error` globally
  to also write here, so **no call site needs to be touched** — still prints
  to the real console too (a dev running from a terminal shouldn't need to
  tail a file).
- **`init()` also bootstraps `failures.cjs` and the compaction cache** — it's
  the single startup entry point that: (a) points the text log at
  `<root>/logs` or the userData fallback, (b) points `failures.useDirectory`
  at `<same dir>/failures`, (c) points the session-compaction cache at
  `<root or userData>/cache/compactions` and sweeps it, (d) loads the
  workspace's secret **values** (not names) and hands them to
  `failures.useSecrets(values)` so the exact-match redaction pass in §5.3 has
  something to match against from process start. **A Rust port's
  initialization sequence must preserve this ordering** — failures'
  redaction is only as good as the secrets it's been given, and those come
  from this init path.
- A workspace chosen/moved **takes effect only at next launch** — the logger
  does not follow a live workspace change mid-session (explicit simplicity
  tradeoff, not a bug).

---

## 6. Rust design notes

### 6.1 Trait seams (mirrors the JS module boundaries, for mockability)

```rust
// Permission engine — pure, no I/O. Mirrors src/shared/permission.js exactly.
trait PermissionEngine {
    fn evaluate(&self, rules: &[Rule], tool: &str, target: &str) -> Verdict; // {action, rule, implicit}
    fn merge(rulesets: &[&[Rule]]) -> Vec<Rule>;
}

// The "ask" half — stateful, async, owns pending questions.
#[async_trait]
trait Approver {
    async fn ask(&self, request: AskRequest) -> Result<Grant, PermissionDenied>;
    fn reply(&self, id: QuestionId, answer: Answer, message: Option<String>) -> ReplyResult;
    fn waiting(&self, session_id: Option<&str>) -> Vec<Question>;
    fn forget(&self, session_id: &str);
    fn revoke(&self, session_id: &str, tool: &str, pattern: &str) -> bool;
}

// Notification sink for pending questions -> UI. Electron's `notify(fn)` becomes
// a channel/event-bus subscription (tokio::sync::broadcast or an mpsc per window).
trait AskNotifier { fn notify(&self, event: AskEvent); }

// Hooks — process spawning + prompt-hook dispatch.
#[async_trait]
trait HookRunner {
    async fn fire(&self, event: HookEvent, ctx: HookContext) -> HookOutcome;
    async fn load(&self, root: &Path, cwd: Option<&Path>, agent: Option<&Agent>) -> LoadedHooks;
}

// Routines — the clock + turn orchestration. TurnRunner is the seam that lets
// this be tested with a fake turn engine, exactly as index.cjs's injected fns do.
trait TurnRunner {
    fn start(&self, cfg: TurnConfig, sender: TurnEventSender) -> TurnHandle;
    fn cancel(&self, id: &TurnId);
    fn read(&self, id: &TurnId) -> Option<TurnRecord>;
}
trait RoutineScheduler {
    async fn tick(&self) -> TickResult;
    async fn run_now(&self, routine_id: &str) -> Result<RunResult>;
}

// Sandbox — one trait, three implementations (docker/daytona/local), matching
// provider.cjs's contract field-for-field.
#[async_trait]
trait ComputerProvider {
    fn id(&self) -> &'static str;
    async fn available(&self, config: &ProviderConfig) -> Availability;
    async fn create(&self, spec: CreateSpec) -> Result<Created>;
    async fn start(&self, handle: &str, ctx: &Ctx) -> Result<Status>;
    async fn stop(&self, handle: &str, ctx: &Ctx) -> Result<Status>;
    async fn remove(&self, handle: &str, ctx: &Ctx) -> Result<()>;
    async fn status(&self, handle: &str, ctx: &Ctx) -> Status;      // never errors — "missing" is a valid answer
    async fn stats(&self, handle: &str, ctx: &Ctx) -> Stats;
    async fn exec(&self, handle: &str, req: ExecRequest) -> Result<ExecResult>; // resolves on nonzero exit; errors only on unreachable
    async fn list_dir(&self, handle: &str, path: &str, ctx: &Ctx) -> Result<Vec<Entry>>;
    async fn read_file(&self, handle: &str, path: &str, ctx: &Ctx) -> Result<String>;
    async fn write_file(&self, handle: &str, path: &str, content: &str, ctx: &Ctx) -> Result<WriteResult>;
    async fn snapshot(&self, handle: &str, ctx: &Ctx) -> Result<Snapshot>;
    async fn snapshots(&self, handle: &str, ctx: &Ctx) -> Result<Vec<Snapshot>>;
    async fn restore(&self, handle: &str, snap_id: &str, ctx: &Ctx) -> Result<RestoreResult>;
    async fn screenshot(&self, handle: &str, ctx: &Ctx) -> Option<String>; // None, never Err, when no display
}

// Support modules
trait FailureLog { fn record(&self, input: FailureInput) -> Option<FailureRecord>; /* never panics/errors */ }
trait Notifier { fn announce(&self, window_focused: bool, payload: Announcement) -> Shown; }
```

### 6.2 Concurrency model

- **Permission `Approver`**: the `pending`/`granted` maps need
  `Arc<Mutex<HashMap<...>>>` or, better, an actor (a single Tokio task owning
  the state, driven by an mpsc channel) so `ask()`'s multi-step cascade
  (check rules → check hook → maybe suspend on a `oneshot::Receiver`) doesn't
  need to hold a lock across an `.await`. Each pending question maps
  naturally to a `tokio::sync::oneshot::Sender<Result<Grant, PermissionDenied>>`
  stored in the pending map, with `reply()` looking it up and calling `.send()`
  — this is a very close match for what a JS `Promise`'s `resolve`/`reject`
  closures already are. Cancellation (`signal.abort`) maps to the caller
  dropping/selecting on a `CancellationToken` (tokio-util) racing the
  `oneshot::Receiver`.
- **Hooks**: `Promise.all` over handlers → `futures::future::join_all` (or
  `tokio::task::JoinSet` if handlers should be independently cancellable).
  Each command handler is a `tokio::process::Command` with
  `tokio::time::timeout` wrapping the wait, and a `CancellationToken` for the
  abort-on-turn-stop path. Process-tree kill on Windows needs an explicit
  `taskkill /pid <pid> /T /F` shell-out (Rust's `Child::kill()` alone does
  not kill descendants on Windows) — must be preserved, this exact command is
  load-bearing.
- **Routines scheduler**: one long-lived Tokio task with a `tokio::time::interval`
  (30s), guarded by an `AtomicBool`/`tokio::sync::Mutex` for the "one tick at
  a time, sequential runs within a tick" invariant. `inFlight: HashSet<RoutineId>`
  behind the same lock (or a `DashSet`) as the authoritative "is this actually
  running in this process" source, independent of persisted `lastRun.status`.
- **Sandbox providers**: Docker/Daytona are I/O-bound (subprocess/HTTP) —
  plain `async fn` over `tokio::process::Command` (Docker) and `reqwest`
  (Daytona) is a direct translation. The Daytona `proxies: Map<handle, base>`
  cache is a `DashMap<String,String>` or `Mutex<HashMap<_,_>>`; same for
  `previewTokens`.
- **Failure log / text log**: both do **synchronous, blocking file I/O by
  design** (the JS version explicitly chose `fs.appendFileSync` for crash
  reliability). In Rust, keep this literally synchronous
  (`std::fs::OpenOptions` + `write_all`) rather than routing through async —
  wrap the shared handle in a `std::sync::Mutex` (a `writing` reentrancy
  guard, as JS has, translates to "the mutex is already held, so a
  panic-during-write can't recurse" — but note the JS guard is really a
  reentrancy-during-single-thread guard; in Rust a `Mutex` naturally subsumes
  that plus gives real thread-safety, which JS didn't need).

### 6.3 Security invariants that must be preserved exactly

1. **A `deny` permission rule can never be overridden** — not by hooks
   (`preapproved`/`forceAsk` only ever *loosen* an `ask`, and the deny check
   in `ask()` happens before any hook or approval-dial logic runs), not by
   the approval dial (`resolveAsk` only turns `ask`→`allow`, never touches
   `deny`), not by an "always allow" grant from a person (a session's granted
   rules are merged *alongside* the base rules with equal-specificity ties
   going to whichever was inserted — a workspace `deny` rule with equal or
   greater specificity than a session grant continues to win via the
   specificity scoring in `evaluate`, since the grant is just another rule in
   the same pool, not a bypass). **`ALWAYS_ASK` (`delete_everything`) can
   never be waived by the approval dial under any dial position, ever.**
2. **`refusedByUser` must be preserved as a distinct signal from a rule-based
   deny** — the agent loop must end the turn on a human's explicit "no"
   (and cascade-refuse the rest of the pending batch in that session) but may
   reason around a rule-based `PermissionDenied` and try something else.
   Collapsing these into one error type loses real product behavior.
3. **Unattended (routine) turns must never leave a permission question
   waiting for nobody** — this was an actual production bug the current code
   fixes explicitly (`unattended: true` → immediate refusal, never a
   suspended promise). A Rust port must not regress to "suspend and hope
   someone answers," which silently hangs a routine to its timeout.
4. **Redaction in `failures.cjs` (§5.3) is mandatory on every free-text field
   before it touches disk**, not best-effort or togglable. Both the exact-
   secret-match pass and every shape-based regex must be ported with
   equivalent coverage; this is explicitly the one thing called out as
   must-not-regress by the task itself.
5. **`local` sandbox provider's path containment (`resolveInside`) is the
   only thing standing between an agent and the rest of the user's disk on
   that provider** — the `..`/absolute-path rejection must be exact; a
   looser Rust reimplementation (e.g. via `std::fs::canonicalize` without
   checking the result still starts with `home`) would be a regression.
   Symlink escapes are a known, currently-unaddressed gap in the source —
   worth a design decision (fix vs. match-parity) but not a silent behavior
   change either way.
6. **Docker/Daytona screen ports must never bind to a non-loopback interface**
   (`127.0.0.1::PORT` for Docker; Daytona's token-gated preview URLs are the
   cloud equivalent of the same policy) — a regression here exposes a live,
   authenticated browser session to the LAN.
7. **`security.cjs`'s origin check for `media`/`clipboard-sanitized-write`
   grants must resolve to the exact built entry HTML file, not a directory
   prefix** — in the Tauri port this becomes "which asset/window origin is
   allowed to request these capabilities," and it must not be loosened to
   "anything served from the app's asset protocol."
