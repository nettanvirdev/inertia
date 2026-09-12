# 03 - Tools: port specification

Source of truth: `D:\oss\inertia\src\main\tools\` (read-only). All file:line references
below are against that tree as it exists at the time of writing. This document is
exhaustive by design: every builtin tool's exact `id`, `description` string and
parameter schema is reproduced verbatim because the model reads these strings to decide
what to do, and a paraphrase changes behavior.

---

## 1. Tool registry

### 1.1 The tool contract (`tool.cjs`)

Every tool, whatever produced it (builtin, MCP, OpenAPI, Composio), is the same plain
object shape, defined once in `src/main/tools/tool.cjs:71-87` via `defineTool(definition)`:

```js
{
  id,            // string, unique across every source, lower-case, what the model calls
  description,   // string, sent verbatim to the model
  parameters,    // JSON Schema object, sent verbatim and used to validate the reply
  permission,    // { key, target(args), always(args) }
  source,        // "builtin" | "mcp" | "openapi" | "composio"
  render,        // (args) => string | null - one-line UI label before there's output
  execute,       // async (args, ctx) => { title, output, metadata, images? }
  normalize,     // optional (rawArgs, ctx) => rawArgs, run before schema validation
}
```

`defineTool` throws at load time if `id` or `execute` is missing (`tool.cjs:72-73`).
Defaults: `description` -> `""`, `parameters` -> `{ type: "object", properties: {} }`,
`permission` -> `{ key: definition.id }`, `source` -> `"builtin"`.

**Request shape for the model** (`toRequestShape`, `tool.cjs:90-99`):

```js
{ type: "function", function: { name: tool.id, description: tool.description, parameters: tool.parameters } }
```

This is the OpenAI-style function-calling envelope; Inertia's LLM adapters (not covered
here) translate it further per-provider as needed.

### 1.2 Registration (`registry.cjs`)

Builtins are a literal, ordered array (`registry.cjs:22-75`) — order matters because it
is part of the cacheable prompt prefix (see 1.5). The list, in order:

`read, lsp, ls, glob, grep, write, edit, patch, file_copy, file_move, file_folder,
file_delete, shell, worktree_enter, worktree_exit, shell_list, shell_logs, shell_write,
shell_kill, present, todowrite, present_plan, skill, question, failures, memory_recall,
memory_save, memory_forget, later, computer_observe, computer_act, computer_open,
computer_launch, computer_page_text, computer_run, computer_list, computer_read,
computer_write, browser_navigate, browser_read_page, browser_read_text,
browser_screenshot, browser_click, browser_type, browser_press, browser_evaluate,
browser_console, browser_network, terminal_read, inertia_list, inertia_get,
inertia_save, inertia_remove, inertia_set_picture, inertia_connect_app,
inertia_set_rules`

Two more are appended only when `includeTask` is true (task delegation not disabled at
this depth): `task`, then the crew family `spawn, collect, wait, team, agent_send,
interrupt, followup` (`registry.cjs:119-130`). The group-conversation family `invite,
handover, part` is always appended (`registry.cjs:135`), independent of `includeTask`.

**Dynamic sources** (`registry.cjs:85-112`): MCP servers, OpenAPI imports, and Composio
apps are each `require`'d lazily and defensively — a source that throws while loading is
dropped with a warning (recorded in a module-level `problems` Map) rather than crashing
the turn. Their tools are concatenated and then **sorted by `id`** (`registry.cjs:111`)
so that two requests whose dynamic tool set differs only in discovery order still share
the same cacheable prefix.

**Per-agent filtering** (`forAgent`, `registry.cjs:145-188`):

1. `all(root, { includeTask: mayDelegate(agent, depth) })` collects every candidate tool.
2. Any tool whose `id` starts with `"computer"` is dropped entirely unless
   `agent?.computerId` is truthy (`registry.cjs:167-170`) — an agent with no machine is
   never shown the tool that drives one, because a visible tool will eventually be tried.
3. Every remaining tool is filtered through the permission engine: if
   `permission.evaluate(rules, key, ANY)` resolves to `deny` with a `pattern === ANY`
   (i.e. denied outright, not just for some arguments), the tool is omitted
   (`registry.cjs:169-174`). A narrower deny (specific pattern) leaves the tool visible
   because it can still be used correctly for other arguments.
4. `task`/`spawn` descriptions are rewritten per-call via `describeTask` (`registry.cjs:199-222`)
   to append the literal list of agents this caller may actually delegate to (name +
   first 200 chars of role/description), because the model needs to read what each
   teammate is *for* and because an agent it cannot delegate to should not be named at
   all (a refusal it had no way to predict is a wasted turn).

Also exported: `invalidate()` (`registry.cjs:225-233`), which asks each dynamic source to
drop its cache — called when an integration changes.

### 1.3 JSON Schema helpers and validation (`schema.cjs`)

The schema *is* the definition — there is no separate validation DSL. Builders
(`schema.cjs:16-55`):

```js
str(description, extra)      -> { type: "string", description, ...extra }
num(description, extra)      -> { type: "number", description, ...extra }
int(description, extra)      -> { type: "integer", description, ...extra }
bool(description, extra)     -> { type: "boolean", description, ...extra }
enumOf(values, description)  -> { type: "string", enum: values, description }
arrayOf(items, description)  -> { type: "array", items, description }
object(properties, required = [])
  -> { type: "object", properties, required, additionalProperties: false }
record(description) -> { type: "object", description, additionalProperties: true }
```

`object()` always sets `additionalProperties: false` — models improvise fields and
unknown keys are dropped rather than failing the whole call. `record()` is the escape
hatch for payloads whose key set is not fixed in advance (e.g. `inertia_save`'s
`fields`), where dropping unknown keys silently would let a caller believe it wrote data
that was actually discarded.

**`validate(schema, value, path)`** (`schema.cjs:72-174`) walks the schema and the raw
value together, producing `{ ok: true, value }` (a *cleaned copy*, defaults filled in,
absent-optional treated same as explicit `null`) or `{ ok: false, error }` where `error`
is an English sentence written for a language model to act on (names the field, e.g.
`` `filePath` must be a string, got number. `` or `Missing required argument \`x\`.`).
Coercions applied: string accepted for number/boolean fields when trivially convertible;
a scalar where an array was expected is wrapped in a single-element array; numeric
strings are parsed.

**`parseArguments(raw)`** (`schema.cjs:187-204`) recovers a provider's raw
tool-call-arguments string, which is nominally JSON but in practice sometimes truncated
or wrapped in a ```` ```json ```` fence. It tries the raw text, then the text with a
leading/trailing code fence stripped, and gives up with
`"The arguments were not valid JSON."` if neither parses to an object.

### 1.4 Running a call (`runTool`, `tool.cjs:109-192`)

This is the single call path used for every tool from every source. Steps, in order:

1. **Normalize** — if `tool.normalize` exists, call it with `(rawArgs, ctx)`; on throw,
   fall back to the raw args unmodified (the schema will explain what's wrong).
2. **Validate** — `validate(tool.parameters, raw)`. On failure, return immediately
   (no permission ask, no execute) with:
   ```js
   { ok: false, title: tool.id, output: `Invalid arguments for ${tool.id}: ${error}`,
     metadata: { error: "invalid-arguments" }, durationMs }
   ```
3. **Ask permission** — compute `target = tool.permission.target?.(args)` and
   `always = tool.permission.always?.(args)`, then
   `await ctx.ask({ key, target: target ?? "*", always, title: tool.render?.(args) ?? tool.id, args })`.
   This throws `PermissionDenied` (a `ToolError` subclass, `retryable: false`) if refused.
4. **Execute** — `await tool.execute(args, ctx)`.
5. **Truncate** — `truncate.output(result.output, { root: ctx.root, toolId: tool.id })`
   (see §2 result envelope).
6. **Return** the success envelope, or catch and return the error envelope. Only
   cancellation (`ctx.signal.aborted`) re-throws; every other failure — including a
   thrown `ToolError` — becomes an `ok: false` result so the model reads it and can react.

---

## 2. Tool result shape (canonical envelope)

**What `execute` returns** (informal, per-tool contract, `CONTRACT.md:64-73`):

```js
{ title: "short label", output: "text the model sees", metadata: { ... }, images?: [dataUrl, ...] }
```

- `output` is *always text* — a tool result is a chat message on every provider worth
  supporting, and none accepts an image inside a `role: "tool"` message.
- `metadata` is for the UI only; the model never sees it. Structured/rich data (diffs,
  file paths, exact byte counts, image data URLs for rendering) goes here.
- `images` (optional array of data-URL strings) is for pictures the *model itself*
  should see. `runTool` turns these into a following user message — the one shape every
  vision-capable provider accepts (`tool.cjs:157-166`).

**What `runTool` actually returns** (the wire envelope every caller of the tool layer
consumes, `tool.cjs:153-191`):

```js
// success
{
  ok: true,
  title: string,
  output: string,             // possibly truncated (see below)
  images?: string[],          // filtered to non-empty strings only
  metadata: {
    ...tool-provided metadata,
    ...(truncated ? { truncated: true, outputPath: string, bytes: number } : {}),
  },
  durationMs: number,
}

// invalid arguments (schema failure — never reaches execute)
{ ok: false, title: tool.id, output: "Invalid arguments for <id>: <msg>",
  metadata: { error: "invalid-arguments" }, durationMs }

// runtime failure (thrown ToolError, PermissionDenied, or any other error)
{
  ok: false,
  title: tool.render?.(args) ?? tool.id,
  output: error.message,
  metadata: {
    error: error.name === "PermissionDenied" ? "denied" : "failed",
    retryable: error.retryable !== false,
    ...(error.refusedByUser ? { refused: true } : {}),
  },
  durationMs,
}
```

Cancellation (`ctx.signal.aborted`) is the **one** case that re-throws instead of
returning an `ok: false` envelope — the turn is over, there is no model waiting to read
the result.

### 2.1 Output truncation (`truncate.cjs`)

`truncate.output(text, { root, toolId, limit = DEFAULT_LIMIT })` (`truncate.cjs:187-225`):

- `DEFAULT_LIMIT = 30000` characters (`truncate.cjs:20`, "roughly 8k tokens").
- If `text.length <= limit`, return unchanged: `{ content: full, truncated: false, outputPath: null, bytes: full.length }`.
- Otherwise:
  1. Try `shrinkJson(full, limit - 400)` — if the text parses as JSON, produce a
     structurally-intact shortened version by clipping long strings first (tried at caps
     `[8000, 4000, 2000, 1000, 500, 250, 120, 60, 30]` characters per string), then
     clipping array item counts (`[40, 20, 10, 5, 3, 1]`) only if string-clipping alone
     can't fit the budget (`truncate.cjs:140-159`). This keeps every key and every
     record shape intact so the result still parses and renders as a structured card
     rather than a wall of text.
  2. If not JSON (or it can't be shrunk to fit), fall back to `pinch(full, limit)`
     (`truncate.cjs:80-84`): keep the first 60% and last 40% of the budget, with a
     `"... [N characters omitted] ..."` marker in the middle — head *and* tail, because a
     test run's failures are at the bottom.
  3. If `root` is provided, the **full, untruncated** text is spilled to
     `<root>/files/tool-output/<toolId>-<timestamp>-<rand>.txt` (written via
     `spillable()`, which pretty-prints JSON with 2-space indent so `read`'s
     offset/limit paging — which pages by line — actually works on it). The notice
     appended to the truncated content tells the model the exact absolute path and says
     to use `read` with an offset to see the rest.
  4. Retention: spilled files are swept (best-effort, never awaited, never blocks the
     tool result) after `RETENTION_MS = 7 days`, at most once per `SWEEP_EVERY_MS = 1 hour`
     (`truncate.cjs:34-71`).

A tool that has **already** truncated its own output (shell, for instance — see §3.6)
sets `metadata.truncated` itself and `runTool`'s generic truncation pass is effectively a
no-op on it (the shell tool truncates to `DEFAULT_LIMIT` before returning, so the
second pass sees text already under the limit).

### 2.2 Error representation

Two error classes, both defined in `tool.cjs:38-52`:

- **`ToolError(message, { retryable = true })`** — "something the model should see and
  could recover from": a missing file, a bad pattern, a non-zero exit code. Caught by
  `runTool` and turned into an `ok: false` result; the model reads `error.message` as the
  tool's `output` and can retry with a fixed call.
- **`PermissionDenied(message)`** extends `ToolError` with `retryable: false` — "the user
  said no. Not the model's fault, and it must not try again." A `refusedByUser` flag
  (set by the permission layer, not the tool) additionally marks the case where a human
  clicked "reject" as opposed to a standing rule denying it — `session`/loop code (not
  covered here) treats a human refusal as ending the turn's remaining tool-call queue,
  while a rule-based deny is something the model can work around.

Error messages are written **for the model**, per house rule in `CONTRACT.md:82-83`:
`"File not found: /x/y.js"` is actionable; `"ENOENT"` is not.

### 2.3 UI-facing metadata worth preserving in a Rust port

- `write`/`edit`/`patch` put a **unified diff** in `metadata.diff` (and, for `patch`,
  one diff per file in `metadata.files[].diff`) — this is what the UI renders as a diff
  card, computed *before* the permission ask so the approval prompt can show the actual
  change (see §4).
- `shell` puts `command`, `exitCode`, `durationMs`, `truncated`, `outputPath`,
  `background`, `pid`, `timedOut`, and (when applicable) `dangers` in metadata.
- `computer_*` and `browser_*` tools put `screenshot`/`closeup` data URLs and page/window
  metadata for the UI, separately from the `images` array that goes to the model.
- `todowrite`/`present_plan` put the structured list/plan in metadata so the UI can
  render a live checklist/plan card while the call is still open (`ctx.update()` is
  called mid-execution for this — see §2.4).

### 2.4 Streaming partial state (`ctx.update`)

`ctx.update({ title?, metadata?, output? })` lets a tool push incremental state to the UI
*while `execute` is still running*, before the final result exists. Used by:
- `shell` (`runAndWait`): every `UPDATE_INTERVAL_MS = 200ms`, pushes
  `{ metadata: { output: chunks.slice(-LIVE_TAIL) } }` where `LIVE_TAIL = 30000` chars —
  so a long-running command's tail is visible live without waiting for exit.
- `todowrite`/`present_plan`: push the list/plan immediately so the card renders before
  the tool call even resolves.
- `computer_run`: streams `onOutput` chunks as they arrive.
- `spawn`/`task`/`followup`: push `{ runId, agent, state: "running", ... }` so a
  delegated run's presence in the UI appears immediately, not just once it settles.

This is a fire-and-forget channel — no acknowledgement, no backpressure contract implied
by the source; a Rust port can model it as an `mpsc` sender captured in the execution
context.

---

## 3. Per-tool deep dive

Every builtin tool is documented below in the order it is registered. Descriptions are
quoted verbatim (long ones summarized with the exact opening reproduced) since these
strings are product-critical. File:line points at the primary source file.

### 3.1 `read` (`builtin/read.cjs`)

**Description** (verbatim, `read.cjs:184-188`):
> "Read a file from disk with line numbers, or list a directory. Reads up to 2000 lines
> at a time; use offset and limit to page through a longer file. Always read a file
> before editing it. An image file - png, jpg, gif, webp - is shown to you as a
> picture: this is how to look at a screenshot, a design or a photo on disk. Do not
> open an image in a browser or on a computer to see it."

**Parameters**: `object({ filePath: str, offset: int, limit: int }, ["filePath"])`.

**Permission**: `key: "read"`, `target: args.filePath`, `always: () => "*"` (a read
grant is always remembered as "read anything," since reading is low-danger).

**Behavior**:
- Resolves `filePath` against `ctx.cwd`; calls `askIfOutside` first (see §5).
- If the path is a directory: lists entries (dirs get a trailing `/`), sorted, paged by
  `offset`/`limit`, wrapped in `<path>/<type>directory</type>/<entries>...</entries>`.
- If it's an image (`.png .jpg .jpeg .gif .webp`, `IMAGE_TYPES` map), and
  `stat.size <= MAX_IMAGE_BYTES = 5MB`: reads the bytes, returns a text note plus
  `images: [data:<mime>;base64,...]` — this is the **only** way an agent "sees" a
  picture; the tool explicitly tells the model not to open images via browser/computer.
  Over 5MB throws a `ToolError` naming the actual size.
- Binary files: refused via extension blocklist (`BINARY_EXTENSIONS`: zip, tar, gz, exe,
  dll, so, class, jar, 7z, doc(x), xls(x), ppt(x), pdf, bin, dat, o, a, lib, wasm, pyc,
  mp3, mp4, mov, woff(2), ttf) **or** content sniffing (`sniff()`: reads first 4096
  bytes, `looksBinary()` returns true on any NUL byte or >30% control characters).
- Text files: split into lines (trailing-newline artifact stripped), numbered
  `"<n>: <content>"`. Per-line cap `MAX_LINE_LENGTH = 2000` chars (longer lines are
  truncated with a `"... (line truncated to 2000 chars)"` suffix). Overall cap
  `MAX_BYTES = 50KB` computed by summing `Buffer.byteLength` per rendered line — stops
  *before* exceeding the cap so the promise is about what came back, not what was
  skipped. `DEFAULT_LIMIT = 2000` lines if `limit` omitted. `offset` beyond EOF throws
  `` `Offset ${offset} is out of range for this file (${total} lines)` ``.
- Empty file: `"(Empty file)"` body.
- **Side effects on success (non-image, non-directory reads)**: calls
  `readState.markRead(sessionId, absPath, stat.mtimeMs)` — this is the *only* way a file
  becomes eligible for `edit`/`patch` (see §3.4/§3.5's read-before-write rule) — and
  fire-and-forgets `lsp.warm(absPath, { cwd })` to pre-index the file for a language
  server so a subsequent `edit`'s post-write diagnostics answer fast.
- **Missing-file UX**: `notFound(abs)` lists up to 3 sibling filenames whose lowercase
  form contains, or is contained by, the requested basename's lowercase form
  (`suggestions()`), appended as "Did you mean one of these?".

### 3.2 `write` (`builtin/write.cjs`)

**Description** (verbatim, `write.cjs:190-192`):
> "Write a file, creating it and any missing parent directories, replacing anything
> already there. Prefer the edit tool for changing part of an existing file."

**Parameters**: `object({ filePath: str, content: str }, ["filePath", "content"])`.

**Permission**: `key: "edit"` (deliberately shared with `edit`/`patch` — "a user who
decided this agent may change files means all the ways it changes files"),
`target: filePath`, `always: filePath` (remembered per-exact-path, not a wildcard).

**Behavior**:
1. Resolve path, `askIfOutside`, `refuseIfProtected` (refuses to overwrite a protected
   Inertia record — see §5.2).
2. Reads previous content if the file exists (`existed` flag) to compute a **unified
   diff** (hand-rolled Myers-style LCS, `unifiedDiff()` — head/tail common-prefix peeling
   then an `O(nm)` DP table capped at `MAX_LCS_CELLS = 4,000,000` cells, beyond which it
   degrades to "everything replaced" rather than exhausting memory; 3 lines of context
   either side, `CONTEXT = 3`) — computed **before** the write, so `metadata.diff` is
   ready when the permission ask fires (the ask itself is `runTool`'s generic ask, which
   fires before `execute` even starts — meaning for `write` the diff card the UI shows
   must actually be produced separately/earlier by whatever renders the permission
   prompt, since `execute` hasn't run yet at ask-time; in practice the UI shows the diff
   post-hoc from `metadata.diff` once the call resolves, or a caller that wants
   pre-approval diffs must precompute one — see the Rust design note in §6 about this
   ordering subtlety).
3. `fsx.writeText` — write-to-temp-then-rename, so a crash mid-write leaves the old file
   intact.
4. Runs the project formatter (`formatFile`, §3.4a) *after* the write.
5. `readState.markRead(sessionId, abs)` — a file just written is known-current, so
   `edit`'s read-before-write gate is satisfied for free.
6. Runs `lsp.report(abs, { cwd, others: lsp.MAX_OTHER_FILES })` — diagnostics for this
   file *and up to `MAX_OTHER_FILES` other files* (a whole-file replace can break
   importers elsewhere that the model has no reason to be looking at).
7. Output: `"Wrote N lines to <path>"` or `"Created <path> with N lines"`, plus a
   formatter note (`formatNote()`, §3.4a) and diagnostics text appended.
8. `metadata: { path, existed, diff, additions, deletions, formatted? }`.

### 3.3 `edit` (`builtin/edit.cjs`)

**Description** (verbatim, `edit.cjs:46-49`):
> "Replace exact text in a file. The file must have been read in this conversation
> first. oldString must match what is in the file, including indentation, and must
> identify one place unless replaceAll is set."

**Parameters**: `object({ filePath: str, oldString: str, newString: str, replaceAll: bool(default false) }, ["filePath","oldString","newString"])`.

**Permission**: `key: "edit"`, `target: filePath`, `always: filePath`.

**Preconditions enforced, in order** (`edit.cjs:69-91`):
1. `askIfOutside`, `refuseIfProtected`.
2. Path must exist and not be a directory (`ToolError`s otherwise).
3. **`readState.wasRead(sessionId, abs)`** must be true, else:
   > "File <abs> has not been read in this conversation. Use the read tool first so the
   > edit is based on what the file actually contains."
4. **`readState.isStale(sessionId, abs, stat.mtimeMs)`** must be false (mtime recorded
   at read time must be >= current mtime), else:
   > "File <abs> has changed on disk since it was read. Read it again before editing."

**Line-ending / BOM handling** (`edit.cjs:94-104`): a leading BOM (`\uFEFF`) is stripped
before matching and restored on write; if the file uses CRLF, content is normalized to
LF for matching (the match ladder in `replace.cjs` reasons in LF) and the result is
converted back to CRLF before writing. `oldString`/`newString` from the model are also
CRLF-normalized to LF first.

**Matching**: delegates to `replace(content, oldString, newString, { replaceAll })` from
`src/main/tools/replace.cjs` — see §3.3a below for the full match-ladder semantics. Any
failure from `replace()` is re-thrown verbatim as a `ToolError` (its messages are
already written for the model).

**On success**:
- Computes diff via `unifiedDiff` (same function as `write`), *before* writing, so it can
  be shown.
- Writes with correct EOL/BOM restored.
- Runs the formatter, then re-marks the file as read (with the **post-formatter** mtime,
  so the model's own next edit isn't rejected as stale by the formatter's own write).
- Runs `lsp.report(abs, { cwd })` (this file only, unlike `write`'s multi-file report,
  because an in-place edit is presumed narrower in blast radius).
- Output: `` `Applied edit to <abs> (<strategy> match, <count> replacement(s))` `` + format
  note + a **capped diff** (`capDiff`, `MAX_DIFF_LINES = 100`: shows first 100 lines then
  `` `... (${N} more diff lines)` ``) + diagnostics.
- `metadata: { path, diff (full, uncapped), strategy, replacements, formatted? }`.

#### 3.3a `replace.cjs` — the match ladder (shared by `edit` and `patch`)

This is the single most important fuzzy-matching algorithm in the tool layer and is
explicitly called out by the task brief as needing exact-match/uniqueness rules
documented. `replace(content, oldString, newString, { replaceAll = false })`
(`replace.cjs:238-296`):

- If `oldString === newString`: refused —
  `"oldString and newString are identical, so there is nothing to change."`
- If `oldString === ""`: refused — `"oldString is empty. Use the write tool to create a file."`
- Otherwise tries each **rung** of the ladder **in order**, stopping at the first rung
  that produces at least one match:

  | # | name | what it matches |
  |---|------|------------------|
  | 1 | `exact` (`simple`) | literal substring, via repeated `indexOf` |
  | 2 | `line-trimmed` | same lines, each side's leading/trailing whitespace ignored |
  | 3 | `whitespace-normalized` | every run of whitespace anywhere collapsed to one space on both sides, then substring match, mapped back to original offsets |
  | 4 | `indentation-flexible` | same lines with a **uniform common-indent prefix** stripped from both sides (preserves *relative* indentation inside the block — distinct from rung 2, which ignores all whitespace per-line) |
  | 5 | `escaped` | first unescapes `\n \t \r \' \" \` \\` in the needle (models sometimes send literal backslash-n instead of a newline), then retries rungs 1 and 2 on the unescaped needle |
  | 6 | `block-anchor` | only for 3+ line needles; finds a line whose trimmed text equals the needle's trimmed first line — **and there must be exactly one such candidate start** (`starts.length !== 1` aborts this rung) — then scans forward for a line matching the trimmed last line |

- For whichever rung produces matches, **overlapping ranges are collapsed** to keep only
  non-overlapping ones (sorted by start, greedy).
- **Disproportionate-match guard** (`disproportionate()`, `replace.cjs:210-217`): any
  candidate match is rejected if it's wildly larger than what was asked for —
  `matchLines >= max(oldLines + 3, oldLines * 2)`, or (for multi-line needles) the
  matched character length exceeds `max(askedChars + 500, askedChars * 4)`. This exists
  specifically because rung 6 (block-anchor) can otherwise swallow a 200-line function
  when a 3-line needle's first/last lines happen to repeat. On trigger:
  > "The text matched a span much larger than what you asked to replace. Read the file
  > again and give the exact text, including whitespace."
- **Uniqueness rule**: if more than one non-overlapping match survives and
  `replaceAll` was not requested, the edit is refused:
  > `` `Found ${N} matches for that text. Include enough surrounding lines to identify
  > the one you mean, or set replaceAll to true.` ``
  Ambiguity is a hard failure, never resolved by picking the first match.
- If **no rung** produces any match at all:
  > "Could not find that text in the file. Read the file again - it may have changed,
  > and the text must match what is actually there."
- On success: `{ ok: true, content, strategy: <rung name>, count: <replacements made> }`.

### 3.4 `patch` (`builtin/patch.cjs`)

**Description** (verbatim, `patch.cjs:228-255`, reproduced in full since exact format
rules are load-bearing):

> "Apply a unified diff across one or more files in a single call. Use this instead of a
> run of `edit` calls whenever a change touches more than one file, or more than one
> place in the same file.
>
> The whole patch applies or none of it does. If any hunk fails, nothing is written and
> you are told which hunk failed and why.
>
> Format - a standard unified diff:
>
> ```
>     --- a/src/one.js
>     +++ b/src/one.js
>     @@ -10,6 +10,7 @@
>      function greet(name) {
>     -  return "hi " + name;
>     +  return `hi ${name}`;
>      }
>     --- a/src/two.js
>     +++ b/src/two.js
>     @@ -1,3 +1,3 @@
>     -const one = require("./one");
>     +const { greet } = require("./one");
> ```
>
> Rules:
> - Every file needs a `--- a/path` line and a `+++ b/path` line. Paths may be absolute
>   or relative to the working directory. The `a/` and `b/` prefixes are optional.
> - Every hunk needs an `@@ -old,count +new,count @@` header. The line numbers are not
>   trusted - hunks are located by their context lines - but the two counts must match
>   the number of lines in the hunk body or the patch is refused.
> - Lines in a hunk start with a space (unchanged), `-` (removed) or `+` (added).
>   Include at least two or three unchanged lines either side of a change so the hunk can
>   be found unambiguously.
> - To create a file, use `--- /dev/null` as the first header and give one hunk that is
>   all `+` lines.
> - Deleting a file is not supported. Use the shell for that.
> - Every file you change must have been read in this conversation first, exactly as
>   with `edit`, and must not have changed on disk since."

**Parameters**: `object({ diff: str }, ["diff"])`.

**Permission**: `key: "edit"`, `target: () => "*"` (the top-level ask is intentionally
generic — parsing paths out of the diff to build a target would mean parsing it twice
and still produce an unrecognizable multi-path string), but **per-file** asks are fired
inside `execute` with the real absolute path as target/always (`patch.cjs:317-319`), so
a rule written about one specific file still fires and "always allow edit" still covers
all of them at once.

**Parsing** (`parseDiff`, `patch.cjs:68-164`): hand-rolled unified-diff parser.
- `cleanPath()` strips git's `a/`/`b/` prefix, unquotes a quoted path, strips a
  `diff -u`-style trailing tab+timestamp; `/dev/null` becomes `null` (create/delete
  marker).
- A file entry needs `--- ` immediately followed by `+++ `; both `/dev/null` is a hard
  error ("nothing to do"). `creating = (before === null)`, `deleting = (after === null)`.
- `@@ -old[,count] +new[,count] @@` header regex: `^@@+\s*-(\d+)(?:,(\d+))?\s+\+(\d+)(?:,(\d+))?\s*@@`.
  Omitted counts default to 1 (matches `diff`'s own convention for single-line hunks).
- **Hunk bodies are bounded by the declared counts**, not by scanning for the next `@@`
  or file marker — deliberately, because a blank context line arrives as an empty string
  with no leading space and a naive "stop at first non-` -+` line" scanner breaks on
  real diffs constantly. `\ No newline at end of file` lines are skipped and count toward
  neither side.
- If the actual body line counts don't match the header's declared counts: refused with
  `` `the hunk header promised ${oldCount} old and ${newCount} new lines but the body has
  ${oldSeen} and ${newSeen}. Count the lines again, or leave the counts off.` ``
- A file appearing twice in one diff (same target path) is refused:
  `` `${path} appears twice in the diff. Put all of its hunks under one file header, in
  order.` ``
- Deleting (any file with `after === null`) is refused outright before any work happens:
  > "This patch deletes <path>, which patch will not do. Remove that file from the diff
  > and delete it with the shell if it really should go."

**Applying** (`applyHunks`, `patch.cjs:184-226`):
- For a **creating** file: hunks are concatenated (their `+` lines only) with no search
  at all — trailing newline added if absent.
- For an existing file: hunks apply **sequentially, each against the result of the
  previous one** (so a second hunk's context lines are what the first hunk left behind).
  Each hunk's "before" text (context + removed lines) and "after" text (context + added
  lines) are extracted (`hunkTexts()`), and located via the **same `replace.cjs` ladder**
  used by `edit` — **not** the `@@` header's line numbers, which are explicitly untrusted
  (only used for hunk-body-length validation above).
  - A pure-context hunk (`before === after`) is a no-op, tracked as strategy `"no-op"`
    (some generators emit these; refusing them would fail an otherwise-correct patch).
  - A hunk with `before === ""` (add-only, no context) can't be anchored:
    > "the hunk only adds lines and gives no context, so there is nowhere to put them.
    > Include the surrounding unchanged lines."
  - Any `replace()` failure surfaces as
    `` `${abs}: hunk ${n} of ${total} (\`${header}\`) did not apply - ${why}` `` where
    `why` is `replace()`'s own error text.

**Per-file preconditions** (identical spirit to `edit`, checked for every file before
*any* write happens — see "read everything, decide everything, write nothing" phase,
`patch.cjs:306-384`):
- `askIfOutside`, `refuseIfProtected`, then per-file permission ask.
- Creating a file that already exists: failed. Non-creating file that doesn't exist,
  is a directory, wasn't read this session, or is stale on disk: each failed with the
  same messages `edit` uses.
- A hunk set that changes nothing (`applied.content === content`): failed —
  `` `${abs}: the hunks changed nothing. The file may already be in the state you
  wanted.` ``

**All-or-nothing semantics**: if `failures.length` is nonempty after the planning pass,
**nothing is written**:
> `` `Nothing was written. ${N} of ${total} file(s) in this patch could not be applied:
> \n\n<bulleted failures>\n\nFix the diff and send the whole thing again.` ``

If planning succeeds for every file, writes happen in a second pass. **If a write itself
throws partway through** (filesystem-level failure, not a patch-logic failure — read-only
file, full disk), everything already written in this call is rolled back in reverse
order (created files unlinked, modified files restored to their pre-patch content read
earlier); if a rollback write itself fails, that's reported too so the tree's actual
state is knowable:
> `` `Writing ${path} failed: ${error}.` `` + either `" Nothing was left changed."` or a
> list of files that could not be restored.

**Post-write** (all succeeded): formatter run per file, `readState.markRead` re-stamped
per file with the post-format mtime, then `lsp.report` per file — **after** the
formatting loop for every file, not interleaved, because a diagnostic taken mid-format
would report errors the patch is in the process of fixing.

**Output**: `` `Applied ${N} file(s):` `` + one line per file (`created` or
`<n> hunk(s)`) + formatter notes + diagnostics blocks.
**metadata**: `{ files: [{ path, created, hunks, diff }] }`.

### 3.5 `ls` (`builtin/ls.cjs`)

**Description** (verbatim, `ls.cjs:42-44`):
> "List the contents of a directory as a tree. Skips node_modules, .git and other
> generated folders. Use depth for a couple of levels at once."

**Parameters**: `object({ path: str, depth: int(default 1), all: bool(default false) }, [])`.
**Permission**: `key: "read"`, `target: path ?? "."`, `always: "*"`.

**Behavior**: `askIfOutside`; refuses non-directory targets. `SKIP` set = `node_modules,
.git, dist, build, .next, target, __pycache__, .venv` — reported (not silently dropped)
in a `(Skipped: ...)` footer line so the model isn't misled into thinking a skipped
folder doesn't exist. `MAX_DEPTH = 3` (clamped), `MAX_ENTRIES = 500` (global cap across
the whole tree, stops recursing once hit). Dotfiles hidden unless `all: true` (but `.git`
is still reported as *skipped*, not silently filtered by the dotfile rule, so its
absence is explained). Output is an indented tree with a trailing `(N entries)` or
`(Stopped at 500 entries.)` footer.

### 3.6 `glob` (`builtin/glob.cjs`)

**Description** (verbatim, `glob.cjs:29-35`):
> "Find files by name pattern. Fast, and it never reads file contents. Supports `*`
> (within one path segment), `**` (across segments), `?` and `{a,b}` alternatives.
> Matching is case-sensitive. Results are the most recently modified matches first,
> capped at 100. Use this when you know something about a file's name or extension; use
> grep when you know something about what is inside it."

**Parameters**: `object({ pattern: str, path: str }, ["pattern"])` — `path` defaults to
`ctx.cwd`.
**Permission**: `key: "glob"`, `target: pattern`, `always: "*"`.

**Behavior**: `askIfOutside`, refuses non-directory target. Walks via `walk.cjs` (§3.6a).
Matches via `matchGlob` (glob syntax detailed below). **Sort order is most-recently-modified
first**, computed over the *entire* match set before truncation — sorting only the first
100 found would silently degenerate into alphabetical order. `MAX_RESULTS = 100`; beyond
that, output ends with a note to narrow the pattern. Empty result: `"No files found"`.

#### 3.6a Glob/walk semantics (`walk.cjs`)

Shared by `glob` and `grep`. No native ripgrep dependency — pure Node `readdir`, by
design (avoids a second cross-platform build matrix and binary distribution problem).

- **`DEFAULT_IGNORE`** (always skipped at any depth, `walk.cjs:36-60`): `node_modules,
  .git, .hg, .svn, dist, build, out, .next, .nuxt, .cache, target, __pycache__, .venv,
  venv, .tox, .gradle, .idea, .vscode, coverage, .DS_Store, vendor, Pods, .terraform`.
- **`MAX_FILE_BYTES = 20MB`** — the point past which a file is no longer "content" for
  any content-reading tool (exported for `grep` to reuse).
- **`.gitignore` support** (root-level only, `parseGitignore`): blank lines, `#`
  comments, leading `/` (anchor to root), trailing `/` (dir-only), `*`/`?` within a
  segment, `**` across segments, leading `!` (negate, last-match-wins), a slashless
  pattern matches at any depth. **Not** supported: nested `.gitignore` files, `\`
  escapes, character classes, re-including a file under an excluded parent (a limitation
  git itself shares). Anything unparseable falls through to *including* the file —
  silently hiding a file is worse than showing an extra one.
- **Symlink-loop protection**: realpaths of entered directories are tracked in a `Set`;
  a symlink pointing at an ancestor is not re-entered.
- **Glob compiler** (`compileGlob`, `walk.cjs:129-158`): `**/` = zero or more whole path
  segments; trailing `/**` = the directory itself and everything under it; bare `**` =
  any characters including `/`; `*` = `[^/]*`; `?` = `[^/]`; everything else escaped
  literally. `{a,b}` brace expansion (`expandBraces`, recursive, handles nesting) happens
  *before* compilation, so the compiler itself never sees braces. Compiled regexes are
  cached per normalized pattern (`globCache`). **Matching is case-sensitive on every
  platform, including Windows** — a case-insensitive match on a case-sensitive question
  is a wrong answer that's hard to notice.
- **`walk(root, { maxDepth, maxEntries, ignore, includeDirs, signal })`** is an async
  generator, depth-first, entries sorted by name for determinism, yielding
  `{ path, relative (always `/`-separated), isDir, size, mtimeMs }`. Checks
  `signal.aborted` before every entry. Unreadable directories are skipped (not thrown).

### 3.7 `grep` (`builtin/grep.cjs`)

**Description** (verbatim, `grep.cjs:69-75`):
> "Search file contents with a regular expression. Returns matching lines grouped by
> file, capped at 100 lines. Matching is case-sensitive unless you set `ignoreCase`.
> Binary files, files over 20 MB, and anything under node_modules, .git or your
> .gitignore are skipped. Use `include` to limit which files are read, `literal` when
> the text you are looking for contains regex characters, and `context` to see the lines
> around each hit."

**Parameters**: `object({ pattern: str, path: str, include: str, literal: bool(default
false), ignoreCase: bool(default false), context: int(default 0, min 0, max 5) },
["pattern"])`.
**Permission**: `key: "grep"`, `target: pattern`, `always: "*"`.

**Behavior**:
- `askIfOutside`. Bad regex syntax throws a `ToolError` with the engine's own message
  plus a suggestion to set `literal: true`.
- **`DEADLINE_MS = 10000`** wall-clock budget for the *whole search*, checked per-file
  entering the walk loop and every 1000 lines within a single file (catches a
  pathological single line, e.g. catastrophic regex backtracking like `(a+)+b`, that a
  per-file check alone would sail past). On timeout, the search stops with whatever it
  has and the result **leads** with an explicit, unambiguous notice — not "no matches
  found" (which would be read by the model as proof of absence):
  > "The search was cut short after 10 seconds and did NOT cover the whole tree, so this
  > is not a complete answer and an absence here does not mean the pattern is absent.
  > Narrow the path, add an include filter, or simplify the pattern and search again."
- `MAX_MATCHES = 100` lines (not files) — truncates with a note to narrow further.
- `MAX_LINE_CHARS = 300` per matched (or context) line; `MAX_CONTEXT_LINES = 5` (context
  param clamped to `[0,5]`).
- Files sniffed for a NUL byte in the first 4096 bytes (`SNIFF_BYTES`) and skipped if
  found (binary). Files over `MAX_FILE_BYTES` (20MB, from `walk.cjs`) skipped without
  reading.
- `include` glob is matched against both the relative path and the bare basename.
- Output format (grep-convention, colon for match / dash for context):
  ```
  <file>:
    Line 12: matched line text
    Line 13- context line
  ```
  Overlapping context between adjacent matches is de-duplicated (a line already printed
  is not printed again) via a `printed` high-water mark per file.
- `metadata: { matches, files, truncated, timedOut, pattern }`.

### 3.8 `write`/`edit`/`patch` shared: project formatter (`format.cjs`)

Not a tool itself but load-bearing behavior threaded through `write`, `edit`, and
`patch`'s post-write step, so documented here for completeness.

`formatFile(absPath, { cwd })` — runs the project's own formatter on a file just written,
**only if the project has evidence it uses one** (a config file present, never just a
binary on PATH). Order matters: **Biome is tried before Prettier** deliberately, because
a project that migrated to Biome often still has a stale `.prettierrc` and running the
tool it migrated away from would silently undo the migration one file at a time.

| formatter | extensions | evidence (any of) | invocation |
|---|---|---|---|
| biome | .js .jsx .mjs .cjs .ts .tsx .mts .cts .json .jsonc .css | `biome.json`/`biome.jsonc` | `biome format --write <file>` |
| prettier | .js .jsx .mjs .cjs .ts .tsx .mts .cts .json .jsonc .css .scss .less .html .vue .md .mdx .yml .yaml | `.prettierrc*`, `prettier.config.*`, or `package.json["prettier"]` | `prettier --write <file>` |
| gofmt | .go | `go.mod` | `gofmt -w <file>` |
| rustfmt | .rs | `Cargo.toml`, `rustfmt.toml`, `.rustfmt.toml` | `rustfmt --edition 2021 <file>` |
| ruff | .py .pyi | `ruff.toml`/`.ruff.toml`, or `pyproject.toml` matching `/^\s*\[tool\.ruff/m` | `ruff format <file>` |
| clang-format | .c .h .cc .cpp .cxx .hpp .hh .m .mm | `.clang-format`/`_clang-format` | `clang-format -i <file>` |

Config is searched up to `MAX_ASCENT = 12` ancestor directories from the file. Binary is
resolved preferring the project's own `node_modules/.bin` (for the `local: true`
formatters — biome, prettier), then PATH; results cached per `(formatter, dir)` for the
process lifetime (`reset()` for tests). `TIMEOUT_MS = 10000` per formatter invocation,
then killed.

**Windows-specific**: a `.cmd`/`.bat` shim cannot be spawned directly since Node closed
CVE-2024-27980, so it's launched via `cmd.exe /d /s /c "<quoted command line>"` with
`windowsVerbatimArguments: true` — the quoting is done once, by this code
(`windowsCommandLine()`), because `cmd /s` strips the outermost quote pair of the whole
tail and a naive per-arg quote leaves the command line torn open at the first space in
any path.

`enabled(cwd)`: disabled by env var `INERTIA_FORMAT` in `{0,false,off,no}`, or by
`.inertia.json` containing `{ "format": false }` in `cwd`.

**Disabled globally**: `env.INERTIA_FORMAT`. **Disabled per-project**: `.inertia.json`
`{ "format": false }`.

`formatFile` **never throws** and never fails the calling edit — a formatter that's
missing, slow, or errors is reported in the tool's output text (`formatNote()`:
`"(Reformatted with <name>.)"` on success-with-change, or
`"(<name> could not format it: <error>. The file was still written.)"` on failure) but
the write/edit itself stands.

### 3.9 File-management family (`builtin/files.cjs`) — `file_copy`, `file_move`, `file_folder`, `file_delete`

Four separate tools sharing one design rationale ("a schema carrying every argument any
action might need reads to a model as a menu, and it picks the wrong item") and one
`edit` permission key (except the directory-delete escalation below).

Shared semantics (`prepare()`, `files.cjs:80-118`):
- **Landing rule**: if `destination` names an existing directory, the effective target is
  `<destination>/<basename(source)>` — "move into it," the way `mv`/every file manager
  behaves; otherwise `destination` is the literal new path.
- **Never overwrites silently**: an existing target without `overwrite: true` is a hard
  refusal naming what's in the way (`"<target> already exists (a folder|<size>). Pass
  overwrite: true to replace it, or choose another destination."`).
- A directory can't replace a non-directory target and vice versa is checked
  (`onto.isDirectory() && !from.isDirectory()` refused).
- A directory cannot be copied/moved **into itself** (`within(source, target)` check).
- Source === target (identical resolved path) is refused.
- Missing parent directories are created automatically (`mkdir -p` semantics).
- All filesystem ops go through `whenReleased()` (from `workspace/fsx.cjs`, not detailed
  here but presumably a lock/queue around concurrent workspace writes).

**`file_copy`** (`files.cjs:135-166`): description: *"Copy a file or a whole folder to
another place on this machine, keeping the original. The destination may be the new
path, or an existing folder to copy into. Creates any missing parent folders. Refuses to
replace something that already exists unless overwrite is true. Use this rather than a
shell command: it works the same on every platform and says what went wrong."*
Params: `{ source: str, destination: str, overwrite: bool }`, both required.
Uses `fsp.cp(source, target, { recursive: isDir, force: true, errorOnExist: false })`.

**`file_move`** (`files.cjs:168-207`): same params/description shape but *"Move or
rename..."*. Tries `fsp.rename` first; on `EXDEV` (cross-device, e.g. across drives)
falls back to copy-then-delete-source, matching what every file manager does. If
replacing an existing target, that target is `rm -rf`'d first.

**`file_folder`** (`files.cjs:209-237`): *"Create a folder on this machine, including
any missing parents. Does nothing if it already exists."* Params: `{ path: str }`.
Refuses if a non-directory already occupies the path.

**`file_delete`** (`files.cjs:239-289`): *"Delete a file, or a folder and everything in
it. This is permanent - nothing goes to the recycle bin - so deleting a folder always
asks first. Pass recursive: true to delete a folder."* Params:
`{ path: str, recursive: bool }`. Single-file delete uses the ordinary `edit` permission
key. **Deleting a directory additionally asks under the `delete_everything` key**
(`ctx.ask({ key: "delete_everything", ... })`, `files.cjs:274-279`) — a key that,
per §4.4, can *never* be pre-approved as "always" and is not waved through even by the
`auto` approval dial. `recursive: true` is required for directories or it refuses with a
one-line prompt to pass it.

### 3.10 `shell` (`builtin/shell.cjs`)

**Description** is assembled dynamically at module load from the actual platform and
shell (verbatim template, `shell.cjs:143-158`):

```
Run a command in the terminal on the user's machine.

Platform: <Windows|macOS|Linux>.
<WINDOWS_NOTES or POSIX_NOTES — see below>

Use this for terminal operations: git, npm, docker, build tools, package managers, running tests.

DO NOT use this to read, write, search for or find files. Use the dedicated tools instead - `read` to read a file, `write` to create one, `edit` to change one, `glob` to find files by name, `grep` to search their contents. They are faster, they do not depend on which utilities happen to be installed, and their output is built for you to read.

The working directory persists between calls. Shell state does not: environment variables, `cd`, activated virtualenvs and shell functions are all gone by the next call, because each call is a fresh process. Use the `workdir` parameter instead of a `cd` command.

Interactive commands will hang until the timeout and then be killed. Stdin is closed, so there is nobody to answer a prompt. Pass the non-interactive flag (`-y`, `--yes`, `--no-input`, `-NonInteractive`) rather than hoping.

Output: stdout and stderr are merged in the order they arrived. Anything over 30000 characters is written to a file and you are given the path. Default timeout 120000 ms, maximum 600000 ms. Use `background: true` for a dev server or a watcher: it returns immediately with a pid, and its output is captured so you can read it later with `shell_logs`.
```

`WINDOWS_NOTES` (`shell.cjs:129-138`) — dynamically states whether pwsh 7 or Windows
PowerShell 5.1 was detected, warns that `&&`/`||` don't exist on 5.1, and gives Unix->PS
command translations (`Get-Content -TotalCount`, `-Tail`, `(Get-Command x).Source`,
`New-Item -ItemType File`, `Remove-Item -Recurse -Force`, `New-Item -ItemType Directory
-Force`, `2>$null`), notes `$env:NAME` not `%NAME%`, and quoting/call-operator advice for
paths with spaces. `POSIX_NOTES` is a one-liner naming the actual shell binary.

**Shell selection** (`pickShell`, `shell.cjs:54-86`): on Windows, prefers `pwsh` (checked
via `onPath()`) over `powershell.exe`, with args
`["-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command"]`.
`-ExecutionPolicy Bypass` is scoped to **this one child process only** and is explicitly
justified in comments: without it, the default Restricted policy blocks `.ps1` shims —
which is how `npm`/`npx` are shipped on Windows — breaking `npm install` outright behind
a confusing error. On POSIX, uses `$SHELL` or `/bin/bash`, with `-c`.

**PowerShell exit-code correction** (`wrap()`, `shell.cjs:105-108`): `-Command`'s own
process exit code reflects *its own* success, not the inner command's; the real code
lives in `$LASTEXITCODE`. The wrapper appends
`\nif ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) { exit $LASTEXITCODE }` so a
native process's real exit code propagates. Guarded on `$LASTEXITCODE` being set, since
pure-cmdlet commands leave it null/stale.

**Parameters**: `object({ command: str, timeout: int, workdir: str, background: bool(default false) }, ["command"])`.

**Permission**: `key: "shell"`, `target: args.command` (the literal command — never
collapsed to the always-pattern first, because a `deny` rule like `shell/rm -rf *` is
matched against this exact target), `always: alwaysPattern(args.command)` (see §3.10a).

**Limits**: `DEFAULT_TIMEOUT_MS = 120000`, `MAX_TIMEOUT_MS = 600000` (clamped, both
directions). `LIVE_TAIL = 30000` chars streamed to the UI mid-run every
`UPDATE_INTERVAL_MS = 200ms`. `GRACE_MS = 3000` between SIGTERM and SIGKILL on POSIX.

**Extra permission asks beyond the standard one** (all fired from inside `execute`,
*after* `runTool`'s generic ask already resolved):
1. **Danger warning** (`shell.cjs:328-350`): if `command.cjs`'s `dangers()` flags the
   command (see §3.10a), a **second** ask fires with `key/target/always` deliberately
   identical to the primary ask (so a cached decision or "always" grant resolves it
   silently) but carrying `metadata: { command, dangers }` so the UI can render the
   specific warning text — the primary ask made by `runTool` has no metadata channel.
2. **Mass-delete guard** (`shell.cjs:366-374`, `key: "delete_everything"`): if
   `massDelete()` flags the command (§3.10a), asks under a key with **no `always`
   parameter at all** — this cannot be pre-approved, ever, by design.
3. **External-directory guard** (`shell.cjs:379-393`): if `externalPaths()` finds the
   command touching a directory outside `cwd`, asks once **per outside directory**
   under `key: "external_directory"`, target/always = `<dir>/*`.

**Background execution** (`args.background === true`, `runDetached`): spawns detached
(`detached: process.platform !== "win32"`), stdio fully piped (unlike foreground, stdin
is **kept** here because a background process is exactly the one legitimately allowed to
stop and prompt — a dev server offering another port, an installer, a REPL — and
`shell_write` answers it). Registered in a module-level `background: Map<pid, entry>`
(not per-session — a dev server outlives the turn that started it). Output tail capped
at `BACKGROUND_TAIL_CHARS = 64KB` per process (drops from the front at a line boundary
when possible, tracking a `dropped` character count so the agent knows the tail isn't the
whole story). Mirrored into a terminal pane tab (`chat:<thread>:job:<pid>`) if the
terminal module is available (best-effort, never required). Finished processes are kept
readable for `FINISHED_TTL_MS = 15 minutes`, capped at `MAX_FINISHED = 20` entries
(`reapFinished()`). `child.unref()` so the app can quit with the child still running;
`killAllBackground()` is called on app quit to actually stop everything.
**Windows kill uses `taskkill /pid <pid> /T /F`** (not `child.kill()`, which on Windows
only kills the shell wrapper and leaves the real process — e.g. holding a port — running).

**Foreground execution** (`runAndWait`): stdin **closed** (`stdio: ["ignore", ...]`) — a
command that stops to ask has nobody to answer, so closing stdin turns a silent hang into
an immediate EOF most tools handle by failing loudly (actionable) rather than hanging
until timeout. `env` additions: `TERM=dumb, NO_COLOR=1, GIT_PAGER=cat, PAGER=cat` (no
color codes, no pager waiting for a keypress that will never come). stdout+stderr merged
in arrival order into one buffer (interleaved errors and progress are the point — a
build's failure sits next to what caused it). On timeout: killed (`softKill` — SIGTERM +
`GRACE_MS` grace then SIGKILL on POSIX; `hardKill`/taskkill immediately on Windows), and
the output is prefixed with `` `The command was killed after exceeding its ${timeout} ms
timeout. If it needs longer and is not waiting for input, retry with a larger timeout.` ``
On non-zero, non-null exit: prefixed with `` `Exit code: ${code}` ``. Cancellation
(`ctx.signal` abort) rejects with the abort reason rather than resolving — this is the
one path that propagates rather than becoming a tool result.

**Output truncation is done locally** (`shell.cjs:665`, `toolId: "shell"`) rather than
relying solely on `runTool`'s generic pass — so `metadata.truncated`/`outputPath` are set
by the tool itself and the notice about where the spill went isn't double-processed.

**`describeSpawnFailure`** (`shell.cjs:558-567`): specifically detects and reports when
`cwd` itself doesn't exist (rather than blaming the shell binary, which is what a raw
ENOENT on the spawned program would otherwise misleadingly suggest — "the computer has
no shell" was an observed real failure mode when this wasn't handled).

#### 3.10a `command.cjs` — command analysis (shared by `shell` for danger/always-pattern classification)

Explicitly **advisory only** — "it shapes the question the user is asked... and never
decides on its own that something is safe." A modest hand-rolled tokenizer, not a real
shell grammar (deliberately, to avoid a WASM/tree-sitter dependency): understands
quoting, escapes, and top-level operators; nothing about subshells, variable expansion,
process substitution, or heredocs.

- **`tokenize(command)`**: splits into `{ tokens, quoted }`, respecting single quotes
  (fully literal), double quotes (backslash escapes a curated `ESCAPABLE` set), and a
  Windows-path-aware backslash rule — a bare backslash only escapes when followed by a
  character that could plausibly need escaping (`'"\ \t&|;<>()$` \``), so `C:\Users\me`
  survives as a literal path rather than becoming `C:Usersme`.
- **`split(command)`**: splits on top-level operators `&& || ; | &` (longest-match-first
  so `&&` isn't read as two `&`), respecting quoting, into `[{ text, operator }]` where
  `operator` is what **follows** each segment.
- **`alwaysPattern(command)`** (`command.cjs:302-314`): the pattern remembered on
  "always allow." Uses an `ARITY` table mapping a command's leading 1-3 tokens to how
  many tokens form the "part a human recognizes" — e.g. `git push origin main` ->
  remembers `git push *` (arity 2 for `git`), `git config` remembers 3 tokens (arity 3),
  while `rm`, `cat`, `curl`, `ssh`, etc. use arity 1 (the argument itself is the dangerous
  part, so remembering `rm -rf *` would remember the wrong half). Falls back to
  `<first-token> *` for anything unlisted. Longest matching multi-word key wins
  (`MAX_ARITY_WORDS = 3`).
- **`dangers(command)`** (`command.cjs:343-379`): matches (whitespace-collapsed,
  lowercased) against a fixed `DANGEROUS` list, each `{ pattern, why }` using the same
  `*`/`?` glob semantics as the permission engine. Entries include: `rm -rf /`,
  `rm -fr /`, `rm -rf /*`, `rm -rf --no-preserve-root*`, `rm -rf ~`, `rm -rf ~/*`,
  `mkfs*`, `dd if=* of=/dev/*`, `:(){*};:` (fork bomb), `chmod -R 777 /`,
  `chmod -R 777 /*`, `curl * | sh`, `curl * | bash`, `wget * | sh`, `wget * | bash`,
  `git push --force*main*`, `git push --force*master*`, `git push -f *main*`,
  `git push -f *master*`, `*drop database*`, `format c:*`, `del /f /s /q c:\*`.
- **`massDelete(command)`** (`command.cjs:401-449`): a *separate* classification from
  `dangers()` — about emptying the **working folder itself**, where an agent otherwise
  has a completely free hand. Checked per chain-segment (so `npm test && rm -rf src`
  doesn't hide behind the first command). Patterns: `rm -r* *`, `rm -fr* *`,
  `remove-item * -recurse*`, `remove-item -recurse*`, `ri * -recurse*`, `rd /s*`,
  `rmdir /s*`, `del /s*`, `del /q*`, `git clean -*d*f*`, `git clean -*f*d*`,
  `git reset --hard*`, `git checkout -- .`, `find * -delete*`, `find * -exec rm*`,
  `*truncate table*`.
- **`externalPaths(command, cwd)`** (`command.cjs:517-556`): only inspects arguments to a
  fixed `FILES` set of path-taking commands (`rm cp mv mkdir touch chmod chown cat type
  del copy move rmdir rd md ren rename get-content set-content remove-item copy-item
  move-item new-item`) to avoid false-positiving on half of every command line. Skips
  flags (`--foo`, single-letter `-x`/`/x`) and any argument containing `$`, a backtick, or
  a glob character in its first path segment (`unresolvable()` — the real value is
  decided by a shell this module doesn't have, and a wrong guess teaches the user the
  prompt is noise). Resolves `~` to `os.homedir()`. Returns the **directory** (not each
  file) outside `cwd`, deduplicated case-insensitively on Windows.

### 3.11 Background-process family (`builtin/shell-background.cjs`) — `shell_list`, `shell_logs`, `shell_kill`, `shell_write`

All four share `shell`'s `key: "shell"` permission (reading/controlling a process you
already started isn't a separate power from starting it).

**`shell_list`**: no params. Lists every tracked background entry as
`<pid>  [running|exited <code>]  <age>s ago  <command>`. Empty: `"Nothing is running in
the background, and nothing recent has finished."`

**`shell_logs`**: `{ pid: int }` required. Returns the captured tail
(`<= 64KB, per BACKGROUND_TAIL_CHARS>`), with a `[<N> earlier characters dropped to keep
this a tail]` note if truncated at the source, further passed through the generic
`truncate.output()` pass (`toolId: "shell_logs"`). Unknown pid:
`` `No background process with pid ${pid}. It may have finished long enough ago to be
forgotten. Run shell_list to see what is still known.` ``

**`shell_kill`**: `{ pid: int }` required. Calls `shell.killBackground(pid)` (which uses
`taskkill /T /F` on Windows, `SIGKILL` on POSIX — the whole process tree). Unknown pid
throws similarly.

**`shell_write`**: `{ pid: int, input: str, newline: bool(default true), end: bool }`.
Types into the process's stdin (a newline appended unless `newline: false`); `end: true`
closes stdin (EOF). Throws if the pipe is already closed ("Its input has already been
closed.") or the pid is unknown/exited.

### 3.12 `worktree_enter` / `worktree_exit` (`builtin/worktree.cjs`)

Git worktree isolation, analogous to Claude Code's `EnterWorktree`/`ExitWorktree`.

**`worktree_enter`**: description (verbatim, `worktree.cjs:134-146`):
> "Work on a copy of this repository, on its own branch, instead of the checkout the
> person is looking at.
>
> Creates a git worktree under `.inertia/worktrees/<name>` on branch `inertia/<name>`
> and moves your working folder there for the rest of this conversation. Every file
> tool and command runs in the copy from then on. Use it for a change that should land
> as a branch rather than as edits in place, for work that must not disturb what the
> person has open, or when several agents will change the same repository at once.
>
> Call `worktree_exit` when the work is committed. If a worktree of that name already
> exists it is reused."

Params: `{ name: str, base: str }`, `name` required. Permission: `key: "shell"`,
`target: "git worktree add <slug>"`, `always: "git worktree *"`.

`ensureWorktree(cwd, rawName, base)` (`worktree.cjs:106-130`, also reused by
`spawn`'s `isolation: "worktree"` option — see §3.14): slugifies the name
(`[a-z0-9._-]`, max 48 chars); resolves the enclosing repo via `git rev-parse
--show-toplevel`/`--git-common-dir`/`--git-dir`; worktree lives at
`<repo-root>/.inertia/worktrees/<name>`, branch `inertia/<name>`. Adds `/.inertia/` to
the repo's `info/exclude` (not `.gitignore`, which would be a tracked change) so the
folder never appears as untracked. If a worktree of that name already exists, it's
reused rather than recreated (`existing: true`). New branch created from `base` (default
`HEAD`) via `git worktree add -b <branch> <dir> <base>`, or an existing branch is
attached via `git worktree add <dir> <branch>`.

**Effect on the turn**: returns `metadata: { cwd: dir, worktree: { path, branch, root,
name }, created }`. Comment notes `cwd` moves the *rest of this turn*, and `worktree`
metadata is what the *conversation* remembers so later turns start there too, until
`worktree_exit` or the agent leaves.

**`worktree_exit`**: description (verbatim, `worktree.cjs:179-184`):
> "Leave the worktree and go back to the main checkout.
>
> The branch is kept - it is the work. Pass `remove: true` to also delete the worktree
> folder, which git refuses while it has uncommitted changes unless `force` is set; say
> what you would lose before forcing anything."

Params: `{ remove: bool, force: bool }`. Throws if not currently in a worktree. With
`remove: true`, runs `git worktree remove [--force] <path>`; on failure (uncommitted
changes without `force`) surfaces git's own message plus guidance to commit/discard or
leave without removing. The branch is **never** deleted by this tool.

### 3.13 Delegation: `task` (`builtin/task.cjs`) — synchronous subagent

**Description** (verbatim, `task.cjs:30-48`):
> "Hand a self-contained piece of work to another agent and get its answer back.
>
> Use it when the work would fill your own conversation with material you do not need
> to keep: searching a large codebase, reading many files to answer one question,
> checking a claim across several places. The subagent does the reading and gives you
> the conclusion.
>
> Do not use it when you already know which file you want - read it. Do not use it for
> a single search - use grep. Do not use it to do work you could do in two tool calls;
> starting a subagent costs a whole conversation.
>
> The subagent cannot see this conversation and cannot ask you anything, so the prompt
> has to stand alone. Say what you want, what it needs to know, and what shape the
> answer should take. Vague prompts come back with vague answers.
>
> Pass a previous task_id to continue that subagent's session rather than starting a
> fresh one, which is much cheaper when you are following up on its answer."

Params: `{ description: str, prompt: str, subagent_type: str, task_id: str }`,
`description/prompt/subagent_type` required. Permission: `key: "task"`,
`target: subagent_type`, `always: "*"`.

**Behavior**: resolves `subagent_type` against the configured `agents` collection by id,
name, or `@handle` (case-insensitive); unknown name lists the actual team roster.
Sessions are cached in a module-level `Map` keyed by a generated `task-N` id (or the
supplied `task_id` if it exists), so `task_id` continuation replays the prior
history + new prompt rather than starting cold. Runs via `session.run()` **at
`depth + 1`**, `unattended: true` is **not** forced here (unlike `spawn`) — actually
looking at the code, `task.cjs` does *not* set `unattended`; it inherits nothing special
(no `unattended` key passed at all, so it defaults false at the session layer — meaning a
subagent spawned via `task` *can* still surface permission asks the ordinary way, subject
to the same `approval`/`signal` the parent turn carries). `depth` and delegation-policy
enforcement (whether nesting is even allowed) happens in `registry.forAgent` (§1.2), not
in this tool. Result text is wrapped as
`<task id="<id>" agent="<name>" state="<stopped>">...</task>`; an empty result throws
`ToolError` naming the stop reason and suggesting a more specific prompt or doing the
work directly. `module.exports.forget(sessionId)` clears cached sessions whose key starts
with the given prefix (session teardown hook).

### 3.14 Delegation: crew family (`builtin/crew.cjs`) — `spawn`, `collect`, `wait`, `team`, `agent_send`, `interrupt`, `followup`

The asynchronous counterpart to `task`: start work **without** waiting, so the caller can
do its own share of work or spawn several things in parallel, then collect answers only
when actually needed. All seven share `key: "task"`.

**`spawn`** — description reproduced in full above the code (`crew.cjs:280-303`); key
points: unlike `task` it returns a run id immediately; names an existing `agent` or
describes a `role` for a **temporary helper** (no disk record, no computer, inherits the
parent's spawn policy so a non-delegating agent can't route around the restriction via a
throwaway helper — `temporaryAgent()`, `crew.cjs:81-91`); optional `isolation:
"worktree"` runs the helper on its own git worktree (reuses `ensureWorktree` from
§3.12, asks `key: "shell", target: "git worktree add <name>"` before creating it);
optional `share_context: true` prepends a digest of the parent's recent steps
(`ctx.recentContext(SHARED_CONTEXT_CHARS = 12000)`) wrapped in
`<parent_context>...</parent_context>` ahead of the brief — otherwise the run sees
**only** its own prompt, nothing of the parent conversation.

Params: `{ description: str, prompt: str, agent: str, role: str, model: str, isolation:
str, share_context: bool }`, `description`/`prompt` required.

**Concurrency policy** (`spawnRefusal`, from `shared/crew.js`, not itself read in full
here but invoked at `crew.cjs:331-338`): computed from `ctx.agentRecord`'s own
spawn-depth/count policy plus hard ceilings — `TEAM_LIMITS`: **20 live, 100 ever per
conversation**, described in the header comment as "not a policy anyone tunes; the wall
a runaway hits."

Each spawned run gets its own `AbortController`, **chained to** (not identical to) the
parent turn's signal — cancelling the parent's turn cancels all its live children, but
stopping one child leaves siblings running. `crew.attach()` also stores closures for
`relaunch` (restart with the same brief) and `followUp` (continue with saved transcript +
new prompt) — kept as closures because a subagent's session needs the provider, API key,
cwd and agent record that only existed in the original tool call, and outlive the run
itself for the panel's restart button.

**`collect`**: description (verbatim key points, `crew.cjs:408-420`): waits for **all**
given `run_ids` **in parallel** (`Promise.all`, not sequential — the whole point).
Params: `{ run_ids: arrayOf(str), timeout_seconds: int }`, `run_ids` required.
`DEFAULT_COLLECT_SECONDS = 600`, `MAX_COLLECT_SECONDS = 1800` (clamped). A run that
failed comes back **as data, not as a thrown error** — "one helper crashing is
information, and you decide what to do about it." Unknown/foreign run ids throw, listing
the actually-known runs in this conversation. Each settled run renders as
`<run id="..." agent="..." status="..." work="...">...</run>` with body = result text,
failure/cancellation reason, or (still running) `Still <activity> - <N> steps so far.`

**`wait`**: waits for *anything* — a spawned run settling, a message arriving in the
inbox, or the person speaking — whichever first (`crew.waitFor()`). `DEFAULT_WAIT_SECONDS
= 300`, same `MAX_COLLECT_SECONDS` ceiling. Distinguishes outcome reasons: `settled`,
`message`, `steered` (person spoke mid-wait — text follows and must be read before
anything else), `timeout`, `cancelled`, or nothing-to-wait-for.

**`interrupt`**: `{ run_id: str, reason: str }`, `run_id` required. Stops a run **mid-turn**
while **keeping its transcript** — distinct from letting it finish or abandoning it.
Its own children keep running (not cascaded).

**`followup`**: `{ run_id: str, prompt: str }`, both required. Continues a **settled**
run (done/failed/interrupted) with its saved transcript intact — not for a still-running
run (`agent_send` is for that).

**`team`**: no params. Read-only status: every run's id/parent/agent/state/description,
plus this run's inbox (drained via `crew.drain()`).

**`agent_send`**: `{ to: str, message: str }`, both required. Fire-and-forget message
into a run's inbox (delivered next time it calls `team`); `to: "parent"` resolves to
`ctx.parentRunId`. Explicitly not a synchronous ask.

### 3.15 Group-conversation family (`builtin/group.cjs`) — `invite`, `handover`, `part`

Distinct in kind from `spawn`/`task`: these change **who is in the current
conversation**, not who does work elsewhere — no session is started, no transcript is
copied or summarized; the invited/handed-off agent simply reads the same transcript
everyone else does. All three share `key: "task"` and all three throw immediately
(`roomOf()`) if the current conversation is not a "group" conversation:
> "This is not a group conversation, so there is nobody to bring in or hand it to. Use
> `spawn` to give a piece of work to another agent instead."

**`invite`**: `{ agent: str, why: str }`, both required. Brings a named colleague in;
they answer **next**, but the caller's own turn is **not** over — "Finish what you were
saying." Colleague resolution (`findAgent`) matches id, name, or `@handle`
case-insensitively; unknown name lists the real roster.

**`handover`**: `{ agent: str, note: str }`, both required. Stronger than invite: the
conversation becomes the named agent's — subsequent person messages address them, not
the caller. The caller may still be invited back or addressed directly but is "no longer
the agent it belongs to."

**`part`**: `{ summary: str, to: str }`, `summary` required. Leaves the conversation
(returns to whoever invited this agent, or `to` if named); the transcript stays visible
to everyone. "The agent the conversation belongs to cannot leave it" (enforced by
`group.leave()`, not shown in this file).

### 3.16 Computer family (`builtin/computer.cjs`) — sandboxed machine

Nine tools sharing `key: "computer"`. Drives the agent's own assigned sandbox — a
*separate* machine from the user's own (contrast with `shell`, which runs where Inertia
itself is running, and `browser_*`, which drives the pane the user is looking at).
`machineFor(ctx)` resolves `sandbox.forAgent(ctx.root, ctx.agentId)`; throws if no
workspace, no assigned computer (`"You have no computer assigned. Ask the user to assign
one on the Computers screen, or use the shell tool to work on their machine instead."`),
or the machine isn't `running`.

- **`computer_observe`**: no params. Screenshot + size + active window + cursor
  position. Uses a **frame-identity cache** (`lastFrame`, keyed by session+machine) so an
  unchanged screen is reported as `(screen unchanged)` text with **no image re-sent** —
  avoids burning tokens on repeated pictures of a static spinner.
- **`computer_act`**: batched, up to `desktop.MAX_ACTIONS` ordered primitive actions
  (`click, move, down, up, type, key, scroll, wait`), then **observes automatically**
  afterward (unless `observe: false`) — described in the header comment as deliberate:
  acting and looking as separate calls "invites the model to act twice before looking
  once." Has a `normalize()` hook (`normalizeActCall`) that tolerates several malformed
  call shapes models actually send (single action object instead of a list, fields at
  the top level, `settleMs` vs `settle_ms`). Returns a **close-up** image of the last
  click location (crosshair overlay) alongside the full screenshot, when a click
  happened, so the model can verify it hit the intended target before typing.
- **`computer_open`**: opens a file path or `http(s)` URL in whatever handles it on the
  sandbox. Explicitly **not** the same as `browser_navigate` — description spells out
  the distinction (separate network/cookies/filesystem from the user's machine).
  Refuses `javascript:`/`data:`/`file:` targets.
- **`computer_launch`**: starts an application (`"browser"` or an arbitrary command
  name) with an optional URI argument.
- **`computer_page_text`**: reads the sandboxed browser's page as text via select-all +
  copy (there's no debugging port on that browser). Has a specific, documented failure
  mode: select-all targets whatever currently holds keyboard focus, so if a text field
  was just typed into, the "page text" returned is actually that field's contents (e.g.
  a password just typed). Detected heuristically (`looksLikeAField`: short, unbroken,
  non-empty text under `FIELD_SUSPECT_CHARS = 120` chars) and flagged with an explicit
  warning appended to the output telling the model to click a blank area and retry.
- **`computer_run`**: shell command on the sandbox. `DEFAULT_TIMEOUT_S = 120`,
  `MAX_TIMEOUT_S = 900`. Non-zero exit is reported as data (`Exit code N.` suffix), not
  thrown — same "let the model see it, don't crash the turn" philosophy as `shell`.
- **`computer_list`** / **`computer_read`** / **`computer_write`**: directory listing,
  text-file read, text-file create/overwrite on the sandbox filesystem.

`observationResult()` (`computer.cjs:110-172`) is the shared renderer for every
tool that returns a screenshot: always states resolution, active window title, and
cursor position as `"the red ring"` (matching the actual overlay drawn on the frame);
states the coordinate-ruler legend every time (not just once in the system prompt)
because it's read alongside the specific picture it describes.

### 3.17 Browser family (`builtin/browser.cjs`) — the in-window pane

Ten tools sharing `key: "browser"`, driving the **same browser pane the person can see**
in the app window — not a hidden/headless instance. This is the explicit design
rationale: an agent claiming "the layout is fixed" about a page nobody can see is
unverifiable; here the person watches the click land and can take the mouse back anytime.
Also explicitly **not** the same surface as `computer_open` (§3.16) — this one has the
user's real cookies and can reach their localhost.

- **`browser_navigate`**: opens a URL, returns page outline (see `browser_read_page`)
  immediately after load rather than making the model ask twice.
- **`browser_read_page`**: structural outline — roles, names, and a `ref_N` per
  interactive element — preferred over a screenshot for text/structure questions;
  `ref_N` values are what `browser_click`/`browser_type` consume. `interactiveOnly` and
  `maxChars` (default 20000, hard cap 50000) params.
- **`browser_read_text`**: readable article/main-content text, for reading rather than
  interacting.
- **`browser_screenshot`**: a picture, for pixel-level questions (spacing, color,
  overlap) — everything else should prefer `read_page`.
- **`browser_click`**: prefers a `ref` (survives scrolling/reflow) over raw `x,y`
  coordinates; fires a **real input event** so hover/focus/pointer handlers all run.
- **`browser_type`**: types into a `ref` (focuses it first) or wherever focus already is.
- **`browser_press`**: a key or chord (`Enter`, `Control+a`).
- **`browser_evaluate`**: runs JavaScript in the page for **debugging/inspection only**
  — description explicitly warns against using it to *implement* changes, since the
  result vanishes on reload; edit source instead.
- **`browser_console`** / **`browser_network`**: console log / network request log since
  last navigation, with `onlyErrors`/`pattern`/`onlyFailed`/`urlPattern`/`count` filters.

**Which tab is driven** (`paneFor`, `browser.cjs:47-50`): the tab the *person* is
currently looking at in *this conversation's thread* — never passed as a tool argument,
so a model can't reach a different conversation's browser by guessing an id. Falls back
to a deterministic default id (`chat:<thread>:web:1`) if no pane is open yet, so calling
a browser tool before the pane is visible doesn't strand a page in an invisible view.
Every tool call, not just navigation, calls `preview.reveal(pane)` first — reading the
console of a page nobody can see is the same underlying problem as loading one nobody
can see.

### 3.18 `terminal_read` (`builtin/terminal.cjs`)

Read-only by explicit design decision (documented at length in the file header): writing
into the person's own terminal was considered and rejected — two writers on one stdin
interleave destructively, and it's the one surface in the app unambiguously theirs.

**Description** (verbatim, `terminal.cjs:23-29`):
> "Read every terminal tab beside this conversation: the commands the user typed, the
> output they saw, and where the shell currently is. Use it when they refer to something
> they ran - "the command I just ran", "this error", "did it pass" - instead of saying
> you cannot see it, and instead of running the command again yourself. Every open tab is
> returned, each with its own folder and whether something is running in it. It runs
> nothing. This is THEIR shell, not yours: use `shell` when you want to run something."

Params: `{ chars: num }` (default 8000, shared across all tabs, not per-tab). Permission:
`key: "terminal_read"`, `always: "*"`. Reads **every** open tab for the thread (not just
the front one — the header comment notes a person watching a build in one tab and a
server in another has two terminals, and an agent that can only see one will confidently
explain a failure using the wrong half). Brings the front-most non-empty tab forward
(`terminal.reveal()`) so the person can watch what's being described. Each tab's content
is fenced and explicitly labeled as untrusted transcript data (a package install script
or test failure is "text from the internet" as often as not) — must be read as a record,
not as instructions.

### 3.19 `lsp` (`builtin/lsp.cjs`)

Single tool, `operation`-dispatched (deliberately — described as Claude Code's/opencode's
precedent — because six near-identical tool descriptions choose badly, unlike the
computer/browser families where the actions are dissimilar enough to need separate
schemas).

**Description** (verbatim key excerpt, `lsp.cjs:42-63`):
> "Ask the language server about the code: where a symbol is defined, who uses it, what
> its type is, what a file contains.
>
> This understands the language; `grep` understands text. Use it whenever the question
> is about a symbol rather than a string - `grep` finds the word in comments and in
> unrelated names, and misses the call that goes through an alias.
>
> Operations:
>   definition        where the symbol at this position is defined
>   type_definition   where its type is defined
>   implementation    what implements it
>   references        everywhere it is used
>   hover             its type and documentation, as the editor would show
>   symbols           everything declared in this file, with line numbers
>   workspace_symbols find a symbol by name across the project (pass `symbol`)
>
> Positions are 1-based, the way `read` numbers its lines: give the line and the
> character where the name starts. Read the file first, so you are pointing at
> something. If there is no language server for this file's language, you are told so
> plainly - fall back to `grep` then, rather than trying again."

Params: `{ operation: enumOf(OPERATIONS), filePath: str, line: int, character: int,
symbol: str }`, `operation`/`filePath` required. Permission: `key: "read"`,
`target: filePath`, `always: "*"`. `MAX_ROWS = 60` locations/symbols returned, with an
`"... and N more"` tail. Explicitly does **not** replace reading — "a definition is a
file and a line; the agent still opens it." No language server for the file's language:
reported plainly (not thrown) with `supported: false` and a nudge to fall back to `grep`
rather than retry. `askIfOutside` before querying.

### 3.20 `present` (`builtin/present.cjs`)

**Description** (verbatim, `present.cjs:44-48`):
> "Show the person the files that are the result of your work, so they can open them
> without reading back through the conversation. Use it when you finish something: the
> page you built, the report you wrote, the picture you produced. Present the finished
> things, not every file you touched on the way - a list of everything is what the
> person already could not read. Any kind of file can be presented."

Params: `{ paths: arrayOf(str), note: str }`, `paths` required. Permission: `key: "read"`
(this changes nothing and reveals nothing the reader couldn't already open, so it's
low-danger by design), `target: paths.join(", ")`, `always: "*"`. `MAX_FILES = 12` — more
than that "is a file listing again, which is the thing being fixed." Refuses to present a
directory (a folder "opens nothing"); missing files are reported inline rather than
failing the whole call (so 4 valid files still present even if a 5th path was a typo).
`metadata.presented` carries `{ path, name, bytes }` per file for the UI's card renderer.

### 3.21 `todowrite` (`builtin/todo.cjs`)

**Description** (verbatim, `todo.cjs:26-47`, reproduced in full — the usage rules are
part of the product contract):
> "Write or update the task list for the work you are doing now.
>
> Use it when a job has three or more real steps, when the user gave you several things
> at once, or when you are partway through something long and need to keep track. Do not
> use it for a single-step task; a one-item list is noise.
>
> Rules that make it useful rather than decorative:
> - Send the WHOLE list every time. This replaces the previous one.
> - Exactly one item may be in_progress at a time.
> - Mark an item completed as soon as it is done, not in a batch at the end.
> - If something turns out not to be needed, mark it cancelled and say why in the
>   content rather than deleting it, so the user can see what you decided.
>
> Use it: "add dark mode, fix the login redirect, and update the readme" is three items,
> written before you touch anything. "Migrate this service to the new client" is one
> sentence and six steps once you have looked at it, so write the list after you look,
> not before.
>
> Skip it: "what does this function do", "rename this variable", "run the tests". A list
> for those is a paragraph of ceremony in front of a one-line answer.
>
> The list is visible to the user as you write it."

Params: `{ todos: arrayOf({ id: str, content: str, status: enumOf(pending, in_progress,
completed, cancelled) }, required: all three) }`, `todos` required. Permission:
`key: "todowrite"` (no target function — every call matches only `*`). **State is
per-session, in-memory only, never persisted** (`Map<sessionId, todos>`) — "a plan is
about this turn, not forever." Refuses more than one `in_progress` item at once. Each
call fully replaces the prior list (not a merge/patch). `ctx.update()` is called with the
list mid-execution so the UI renders it live. The **result's `output` re-states the whole
list** to the model as a checkbox render (`[x]`/`[>]`/`[ ]`) — "that re-reading is most of
the value," keeping the plan the most recently-read thing in the transcript even many
steps in.

### 3.22 `present_plan` (`builtin/plan.cjs`)

Only offered in Plan mode (mode gating happens upstream of the registry, not shown in
this file, but the tool's own comments make clear Plan mode has no other tools that
change anything — this is how a planning turn *ends*).

**Description** (verbatim key excerpt, `plan.cjs:39-58`):
> "Show the user your plan and ask them to approve it. This is how a planning turn ends.
>
> Call it once, when you have looked at enough of the project to write a plan you would
> be willing to build from. The user sees the steps and chooses: build it, which
> switches this conversation to Autonomous mode and starts the work, or keep talking,
> which leaves you here to revise it.
>
> Write steps as work someone could start... Order them so each one can begin when the
> one before it is finished.
>
> Say what you are unsure about in `risks` rather than leaving it out. A plan that hides
> the part you have not figured out gets approved and then fails at exactly that point...
>
> Do not call this and then keep working - you have no tools to work with in Plan mode,
> and the turn is over once the plan is on screen."

Params: `{ title: str, summary: str, steps: arrayOf({ title: str, detail: str },
required: title), risks: arrayOf(str) }`, `title/summary/steps` required. Permission:
`key: "todowrite"` (shared key — same low-danger, informational-card class of action).
`MIN_STEPS = 1`, `MAX_STEPS = 40` — a plan below/above these is refused with guidance
("write what would actually be done, not a heading for it" / group into fewer steps).
Plan is stored per-session (`Map`, in-memory, `planFor(sessionId)` exported for a
follow-up "build" turn to quote back verbatim). `ctx.update({ metadata: { plan,
awaitingApproval: true } })` fires while the call is still open so the card renders
immediately. Explicitly **cannot approve itself** — "It does not switch modes, and it has
no way to. The mode changes when the person presses the button, in the renderer."

### 3.23 `skill` (`builtin/skill.cjs`)

**Description** (verbatim, `skill.cjs:66-77`):
> "Load a skill: a set of instructions the user wrote for a particular kind of work.
>
> The skills available to you are listed in your system prompt with a description each.
> When a task matches one of those descriptions, load it BEFORE you start work rather
> than after - a skill exists because there is something about this task that is not
> obvious, and finding that out afterwards means doing the work twice.
>
> The instructions arrive in the conversation, along with a listing of the skill's own
> folder. Paths mentioned in the instructions are relative to that folder, and you can
> read or run those files with the ordinary tools."

Params: `{ name: str }`, required. Permission: `key: "skill"`, `target: name`,
`always: name` (**per-skill**, not `*` — "agreeing to load the deployment skill is not
agreeing to load every skill that ever gets added"). A skill is a folder
`skills/<slug>/SKILL.md` plus arbitrary sibling files. Resolution tries exact id match
first (unique), then name match (only unambiguous if exactly one); more than one skill
sharing a name is refused rather than silently picking the first
(`"More than one skill is called <name>. Ask for one of these folder names instead:
<ids>."`). Missing `SKILL.md` or `enabled: false` are separately reported. Output wraps
instructions in `<skill name="...">...</skill>` plus a `<skill_files>` listing (capped at
`MAX_FILES = 40`, with a "not listed" tail count if more exist), and — critically — a
statement that any `allowed-tools:` frontmatter the skill declares is **advisory to the
model only and grants nothing**: "Your permission rules are unchanged by loading it." A
skill file in the workspace cannot widen its own agent's actual permissions.

### 3.24 `question` (`builtin/question.cjs`)

**Description** (verbatim, `question.cjs:57-69`):
> "Ask the user a question and wait for their answer, without ending your turn.
>
> Use it only when the answer changes what you would build and you cannot work it out
> yourself. Two designs that are both defensible, a destructive step that is reasonable
> but not obviously wanted, a missing detail that is not in any file.
>
> Do NOT use it for anything you could answer by reading the code, for permission to use
> a tool (tools ask for themselves), or to check in on work the user already asked for.
> When a sensible default exists, take it and say what you assumed.
>
> Give real options where you can. Choosing from three is faster than writing a
> sentence, and the user can always type something else."

Params: `{ question: str, options: arrayOf({ label: str, description: str },
required: label) }`, `question` required, up to 4 options kept. Permission:
`key: "question"` (no target — only ever matches `*`). Distinct from ordinary permission
asks in that this **suspends the tool call on a real answer**, not an allow/deny
decision — a separate `pending: Map<id, {resolve, reject}>` and `notify` callback,
mirroring the shape of `permission/index.cjs`'s own pending-question machinery but for a
different kind of question. If `!notify` (no window) or `ctx.unattended` (routine/crew
run): refuses immediately —
`"There is nobody to ask right now. Make the most sensible choice and say what you assumed."`
On cancellation mid-wait, the pending promise is rejected with a non-retryable
`ToolError` so the tool call doesn't hang forever. On answer, the tool result is
`"The user answered: <reply>"`.

### 3.25 `failures` (`builtin/failures.cjs`)

Read-only log query over `src/main/failures.cjs` (the failure-logging module itself is
out of scope for this doc, but its query surface is exercised here). **Description**
(verbatim key excerpt, `failures.cjs:58-73`):
> "Read the log of things that have gone wrong before: tool calls that failed, commands
> that exited non-zero, refused permissions, provider errors, routine and subagent
> failures, and crashes. Every record has the arguments, the error, the tail of the
> output and the folder it happened in.
>
> Use it when something fails the same way twice, when the user asks why something keeps
> happening or what went wrong earlier, and before retrying a command that you suspect
> has failed before - the log will tell you how it failed last time and what was tried.
> Do not use it for a failure you have just seen once and can fix from the message in
> front of you.
>
> With no filters it summarises the last two weeks: what repeats, then the newest
> records. Narrow with `query` (matched against error, arguments and output), `kind`,
> `tool` or `agent`. Set `verbose` to read full output."

Params: `{ query: str, kind: enumOf(failures.KINDS), tool: str, agent: str, days:
int(default 14, max 90), limit: int(default 20, max 50), verbose: bool(default false) }`.
Permission: `key: "failures"`. Unfiltered queries lead with a **repeats-first summary**
("one failure is an accident; the same failure four times is a fact about the project")
before the newest-first record list — deliberately, so the reader meets the pattern
before the noise. Each row shows timestamp, kind/tool/agent, `seen Nx` if repeated, first
line of the error, cwd, truncated args, and (verbose) full output or (non-verbose) the
last 2 lines of output.

### 3.26 Memory family (`builtin/memory.cjs`) — `memory_recall`, `memory_save`, `memory_forget`

**`memory_recall`** (`key: "memory"`, `target: () => "recall"`): full-text search over
stored memories beyond what's already injected into the system prompt (the prompt only
carries titles, budgeted). Params: `{ query: str, limit: int(default 5, 1-25) }`, `query`
required. An empty result is reported as a **real answer**, not a failure:
`"No memory matches "<query>". You were not told this, so do not assume it."` —
explicitly framed to prevent the model from treating "nothing found" as license to guess.

**`memory_save`** (`target: () => "save"`): description explicitly lists what to save
(how the person wants to be worked with, decisions and reasoning, what a project is for,
corrections) and what **not** to (anything readable from code/git, transient
conversation-only state, anything already in AGENTS.md). "One fact per memory, always" —
`replaces: <id>` supersedes an existing memory instead of creating a duplicate that
contradicts it. Params: `{ title: str, body: str, kind: enumOf(MEMORY_KINDS), scope:
enumOf(MEMORY_SCOPES, default "project"), tags: arrayOf(str), replaces: str }`,
`title`/`body` required. **Secret detection is enforced in the tool itself**
(`shared.looksSecret()`), before storage — a memory is later injected into every prompt
and shown on a screen, so a leaked key here leaks repeatedly:
> "That looks like it contains a key, token or password, so it was not saved. Memories
> are put into later prompts and shown on screen. Save what the secret is for and where
> it lives, never its value."
`scope: "project"` resolves the memory's `folder` to the **repository root**
(`projectRoot(ctx.cwd)`), not the subdirectory the turn happens to be in, so a memory
saved from a subpackage is visible from the whole project.

**`memory_forget`**: `{ id: str }`, required. Uses the **`inertia_guarded`** permission
key (shared with deleting any other workspace record) rather than `memory`'s own key —
deleting is a different, always-asks-by-default decision from reading/writing. Throws if
the memory backend doesn't support single-record deletion.

### 3.27 `later` (`builtin/later.cjs`)

**Description** (verbatim, `later.cjs:44-52`):
> "Come back to something later, after this turn has ended. Writes a one-off routine
> that runs the brief you give it at the time you give it, in a conversation of its own
> under Routines, whether or not this conversation is still open.
>
> Use it for work that has to wait on the world - a deploy finishing, a reply arriving, a
> build that takes an hour - rather than sitting in `wait` for it. The brief is all the
> future run will see, so write it as instructions to someone who has not read this
> conversation: what to check, what to do about each outcome, and what to tell the
> person. Say in your reply that you have scheduled it and when."

Params: `{ brief: str, in_minutes: num, at: str(ISO8601), name: str }`, `brief`
required, exactly one of `in_minutes`/`at`. Permission: `key: "inertia_save"`,
`target: () => "routine"` (this tool writes a real routine record, so it rides on the
Inertia-setup permission key, not a key of its own). `MAX_AHEAD_MS = 366 days`. Writes a
`routines` collection record with `schedule.kind: "once"`, `mode: "autonomous"`, and —
critically — `approval: ctx.approval ?? "auto"`, i.e. **inherits the calling
conversation's approval dial** so the scheduled run refuses the same things the live
conversation would have (rather than silently becoming more permissive). Unattended
routines treat "ask" as "refuse" (see §4.5), which is by design here: a routine scheduled
from a chat set to "ask" will refuse the same calls the chat would have asked about.

### 3.28 Inertia self-management family (`builtin/inertia.cjs`) — `inertia_list`, `inertia_get`, `inertia_save`, `inertia_remove`, `inertia_set_picture`, `inertia_connect_app`, `inertia_set_rules`

Everything Inertia itself shows on screen is a file in the workspace folder, so an agent
with the ordinary file tools *could already* write one — badly, without knowing field
names/id rules/schedule shapes, and with no validation, producing malformed records the
app can't render. This family is a first-class, validated, kind-aware API over the same
records instead of a free-form file write.

**Kinds** (`KINDS` table, `inertia.cjs:79-232`), one entry per record type, each with a
`collection`, allowed `fields` (deliberately **not** every field on the record — e.g. an
agent's `stats`/`lastRun` are app-written and excluded so the model can't fabricate
history), a `validate(record)` function, and a `complete(record)` function that fills in
sane defaults:

| kind | collection | fields the model may set | notable validation |
|---|---|---|---|
| `agent` | `agents` | name, handle, role, description, systemPrompt, model, cwd, computerId, tags, icon, avatarColor, status, thinkingBudget, reasoningEffort | needs a non-empty `name` |
| `routine` | `routines` | name, description, agentId, markdown, schedule, enabled, icon, tags, mode, approval | needs name, agentId, markdown; `mode` must be a valid mode; `approval` must be a valid approval; `schedule.kind` in `cron/interval/once/manual/trigger`; for cron/interval/once, the schedule must actually produce a next run (`schedule.nextRun()`) or is refused |
| `skill` | `skills` | name, description, instructions, enabled, tags, version | needs name, description (with an explicit note that description is the *only* thing another agent sees before deciding to load it — must say what AND when), instructions |
| `memory` | `memory` | title, body, kind, agentId, tags, pinned | needs title, body |
| `computer` | `computers` | (none — **read-only** through this API) | making a machine requires the Computers screen (Docker/Daytona provisioning takes minutes and is out of scope for a tool call) |
| `mcp` | `plugins.mcp` | name, type, command, args, cwd, env, url, headers, enabled, timeoutMs | `live: "mcp"` — goes through the real MCP-server start/stop machinery, not a plain record write |
| `api` | `plugins.openapi` | name, url, text, baseUrl, enabled | `live: "openapi"` — goes through spec import/validation |
| `app` | `plugins.composio` | (none — **read-only**, connecting requires OAuth via `inertia_connect_app`) | |

**`inertia_list`**: `{ kind: enumOf(KIND_NAMES) }`, optional — omitted, returns a
per-kind count object; given, returns a compact per-kind summary array (`summarise()`,
one small object per record — not the full record, since "what agents are there" doesn't
need the full system prompt of each one).

**`inertia_get`**: `{ kind, id }`, both required. Full record (`detail()` — strips
`runHistory` and, for `app`, `composioUserId`).

**`inertia_save`**: `{ kind: enumOf(non-readonly kinds), id: str(optional), fields:
record() }`, `kind`/`fields` required. `id` present = update (must already exist, and
must not be `isProtected()` — see §5.2); absent = create. `fields` sent as a JSON string
by the model is auto-parsed via `normalize()`. Unknown field names are refused, listing
the actual allowed field set for that kind. Live kinds (`mcp`, `api`) route through the
real subsystem functions (`mcp.addServer/updateServer`, `openapi.importSpec`) rather than
a plain collection write, so e.g. saving an MCP server record actually starts the
process. On success, `announce()` fires a workspace-change event so every open
screen/window redraws immediately with no refresh needed (best-effort — swallowed if no
Electron window context, e.g. under unit tests). Permission: `key: "inertia"`,
`target: "<kind>:<id ?? 'new'>"`, **`always: "<kind>:*"`** — the always-grant is scoped
per-kind, not global.

**`inertia_remove`**: `{ kind: enumOf(non-computer kinds), id }`, both required. **One
named record per call — no bulk sweep exists.** Uses the **`inertia_guarded`** key
(distinct from `inertia_save`'s `inertia` key — creation and destruction are explicitly
separate permission decisions). Refuses `isProtected()` records. For an `agent`, also
deletes its picture file. Live kinds route through `mcp.removeServer`/
`openapi.removeImport`/`composio.disconnect`.

**`inertia_set_picture`**: `{ agentId, url: str, path: str, clear: bool }`, `agentId`
required, exactly one of `url`/`path`/`clear`. Fetches (`url`, via `fetch()`, respecting
`ctx.signal`) or reads (`path`, resolved against `ctx.cwd`) an image; validates MIME type
against `png/jpeg/webp/gif`; caps at `MAX_PICTURE_BYTES = 2MB`; stores one file per agent
under `agents/pictures/<agentId>.<ext>` (removes any stale extension variant first).

**`inertia_connect_app`**: `{ toolkit: str, search: str, status: str }`, all optional
(mutually distinguishing modes). No args = browse catalogue (optionally filtered by
`search`, capped at 40 results). `toolkit` given = starts a Composio OAuth connection,
returns a link the model must hand to the person plus the connection id to poll with
`status`. `status` given = polls that connection's state; once `ACTIVE`, the new tools
(`<toolkit>_*`) are noted as available **from the next turn**, not this one (tool
discovery happens per-request, at the top of the loop).

**`inertia_set_rules`**: `{ agentId: str(optional), rules: arrayOf({ tool: str,
pattern: str(default "*"), action: enumOf(allow,ask,deny) }, required: tool+action) }`,
`rules` required. Omitting `agentId` targets the whole workspace's default ruleset;
given, targets one agent's own overrides. Uses the **`inertia_guarded`** key — "this
widens what an agent may do, including what it may do to Inertia itself, so it asks
every time unless the user has said otherwise." Unknown `tool` keys (not in
`TOOL_KEYS`, §5.1) are refused, listing the valid set. Replaces by `(tool, pattern)`
identity rather than appending — writing a rule for a `(tool, pattern)` pair that already
has one **replaces** it (never leaves two conflicting rules for the same question).

---

## 4. Permission integration

### 4.1 Two-layer permission model

`src/shared/permission.js` (pure, dependency-free, also imported by the renderer for the
settings UI) is the **engine**: given an ordered, flat ruleset of `{ tool, pattern,
action }` and a `(key, target)` pair, it returns the winning rule (most-specific match —
tool-name specificity dominates pattern specificity, `specificity()` in
`permission.js:56-61`, so an exact tool-name match always beats a wildcarded tool name
however specific its pattern is) via `evaluate()`. Rulesets merge in order
`defaults -> workspace -> agent's own` (`merge()`, later wins per `(tool, pattern)` key).

`src/main/permission/index.cjs` is the **orchestrator**: what to actually *do* when the
engine says `ask`. It holds `pending: Map<requestId, {question, resolve, reject}>` and
`granted: Map<sessionId, rule[]>` (session-scoped "always" grants — deliberately **not**
persisted to the workspace; a mid-conversation grant is not a permanent settings change
until the user makes it one explicitly elsewhere).

### 4.2 The `ask()` decision cascade (`permission/index.cjs:82-206`)

For a request `{ sessionId, key, target, always, title, agent, signal, metadata }`,
merged against `grantsFor(sessionId)`:

1. **`deny`** -> throws `PermissionDenied` immediately, naming the specific rule if there
   was one (`` `Your rule \`${tool}: ${pattern}\` forbids it.` ``), with instructions to
   the model to *not retry* and instead tell the user.
2. **`allow`** (and not `forceAsk`) -> resolved immediately, no card shown
   (`{ granted: true, asked: false }`).
3. **`preapproved`** flag on the request (set by a caller upstream, e.g. a
   `PreToolUse`-style hook that already looked and said allow) -> resolved immediately.
4. **Approval dial** (`resolveAsk(approval, key)`, `shared/approval.js:91-96`): can only
   loosen an `ask` verdict into `allow` (never overrides a `deny`, never overrides
   `forceAsk`). `auto` allows everything except `doom_loop` (which it **denies** — the
   repeated-identical-call loop guard, §4.6). `edits` allows only keys in `EDIT_KEYS =
   ["edit"]`. `ask` (default) changes nothing.
5. **`beforeAsk` hook** (a `PermissionRequest`-style hook) — can answer `allow`/`deny`
   before a human is asked at all. Deliberately runs **before** the unattended check, so
   a routine with a hook that decides its own permissions doesn't have to refuse
   everything just because nobody's watching.
6. **Unattended turn** (`request.unattended`) -> refused unconditionally with a message
   telling the model to finish the rest of the work and *say in its summary* what it
   wanted to do, so the person can grant it via the routine's agent permissions later.
7. **No `notify` registered** (no window at all) -> refused (headless run must never
   silently take an action configured as needing a look).
8. **Turn already cancelled** (`signal.aborted`) -> refused immediately, no card shown.
9. Otherwise: a real card is created (`ask-N` id), `notify({ type: "asked", question })`
   fires, and the call **suspends on a Promise** until `reply()` is called or the turn's
   abort signal fires.

### 4.3 The approval request shape shown to the UI

```js
{
  id: "ask-<n>",
  sessionId,
  agent,              // whose work this is — most relevant when a subagent is asking
  key,                // the permission key, e.g. "shell", "edit"
  target,             // what the rule is written about — e.g. the literal command or path
  always,             // what "always allow" would remember (shown on the button so the
                       // scope of the grant is visible before agreeing)
  title,              // one-line label, usually tool.render(args)
  metadata,           // tool-specific extras for rendering — e.g. shell's { command, dangers }
  at: Date.now(),
}
```

### 4.4 Reply flow (`reply(id, answer, message)`, `permission/index.cjs:216-251`)

Three answers:
- **`"reject"`**: rejects the suspended promise with a `PermissionDenied` carrying
  `refusedByUser: true` and, if the person typed feedback, that text
  (`` `The user refused this tool call and said: ${message}` ``). **Also cancels every
  other pending question in the same session** (`rejectSession()`) — "one 'no' cancels
  the turn": if a model fired three tool calls and the first is refused, the other two
  are refused too, rather than asking the same underlying "no" three separate times.
- **`"always"`**: appends `{ tool: key, pattern: always, action: "allow" }` to the
  session's `granted` list, resolves the current promise, and re-evaluates every *other*
  still-pending question in the session against the updated grants
  (`settleCovered()`) — so a batch of calls that would now all be covered by the new
  grant resolve together instead of asking again one at a time.
- **anything else (`"once"`)**: resolves just this one call, grants nothing standing.

### 4.5 Unattended / approval-dial integration points

- **`ctx.unattended`**: set true for routine runs and (per `crew.cjs:190-193`) every
  `spawn`-launched run — "a spawned run has no window in front of it. Anything that would
  stop to ask a person refuses with a reason instead." Not set by `task.cjs` (a
  synchronous subagent inherits the parent's own unattended-ness rather than being forced
  unattended, since it runs interleaved with a turn that may itself have a person
  watching).
- **`ctx.approval`**: the conversation's approval dial (`ask | edits | auto`), threaded
  down into every `spawn`/`task`/`followup` child unchanged (children answer to the same
  dial the parent conversation does).
- **`ALWAYS_ASK = ["delete_everything"]`** (`shared/approval.js:72`): no dial position,
  including `auto`, ever waves this key through. The `delete_everything` key itself also
  has **no `always` pattern at all** at the individual-ask call sites (`shell.cjs:366-374`,
  `files.cjs:274-279`) — it cannot be pre-approved even manually, only answered once at a
  time.

### 4.6 The `doom_loop` key

Referenced in `shared/tool-access.js` catalog (`key: "doom_loop"`, `danger: "medium"`,
`fallback: "ask"`) and specifically carved out in `resolveAsk()` — under `auto`, every
other key is waved through but `doom_loop` is **denied** outright. This is the loop guard
for "the model made the exact same call, with the exact same arguments, three times in a
row" (mentioned in `permission/index.cjs:118-122`'s deny message: `` `you have made the
same call with the same arguments ${repeats} times in a row` ``); the detection/counting
logic itself lives in the session loop (outside `src/main/tools/`, not covered in this
document, but the permission-layer hook point is exactly this key).

### 4.7 Every permission key -> tools mapping

Reproduced from `shared/tools.js:37-372` (`TOOLS` array) for completeness, since a Rust
port's permission UI needs this exact grouping:

| key | tools | danger | fallback |
|---|---|---|---|
| `read` | read, ls, lsp, present | low | allow |
| `edit` | write, edit, patch, file_copy, file_move, file_folder, file_delete | high | ask |
| `glob` | glob | low | allow |
| `grep` | grep | low | allow |
| `shell` | shell, shell_write, worktree_enter, worktree_exit (shell_list/shell_logs/shell_kill also key off `"shell"` per their own `permission.key`) | high | ask |
| `computer` | computer_observe, computer_act, computer_open, computer_launch, computer_page_text, computer_run, computer_list, computer_read, computer_write | medium | ask |
| `browser` | browser_navigate, browser_read_page, browser_read_text, browser_screenshot, browser_click, browser_type, browser_press, browser_evaluate, browser_console, browser_network | medium | ask |
| `terminal_read` | terminal_read | low | allow |
| `delete_everything` | (no fixed tool list — raised ad hoc by shell/file_delete) | high | ask, never "always" |
| `external_directory` | (ditto, raised by `askIfOutside`) | high | ask |
| `task` | task (spawn/collect/wait/team/agent_send/interrupt/followup/invite/handover/part also key off `"task"`) | medium | allow |
| `skill` | skill | low | allow |
| `memory` | memory_recall, memory_save | low | allow |
| `load_tools` | load_tools | low | allow |
| `todowrite` | todowrite (present_plan also keys off `"todowrite"`) | low | allow |
| `question` | question | low | allow |
| `doom_loop` | (session-loop-raised, no tool list) | medium | ask |
| `failures` | failures | low | allow |
| `inertia` | inertia_list, inertia_get, inertia_save, inertia_set_picture, inertia_connect_app | medium | allow |
| `inertia_guarded` | inertia_remove, inertia_set_rules, memory_forget | high | ask |
| `mcp` / `openapi` / `composio` | dynamic, discovered at runtime | medium | ask |

`keyForTool(id)` (`tools.js:388-391`) is the fallback lookup any tool without its own
`permission.key` uses; every dynamic-source tool sets its own key explicitly (`mcp`,
`openapi`, `composio`) and is never looked up this way.

---

## 5. Sandbox / boundary interaction

### 5.1 Filesystem boundary: `outside.cjs`

Every file-touching tool (`read`, `write`, `edit`, `patch`, `ls`, `glob`, `grep`,
`file_copy/move/folder/delete`, `lsp`, `present`) calls `askIfOutside(ctx, absPath,
verb)` before doing anything (`outside.cjs:58-78`). No-op if the path resolves inside
`ctx.cwd`. Also no-op if the path is inside the **Inertia workspace folder itself**
(`ctx.root`) but **outside** a small guarded subset
(`GUARDED = ["secrets", "conversations", "history", "backups", "inertia.json"]`,
`outside.cjs:37`) — the rest of the workspace (agents, routines, skills, memory records)
is treated as ordinary working territory an agent is expected to write into, not foreign
soil. Otherwise, asks once **per containing directory** (not per file) under
`key: "external_directory"`, target/always = `<dirname(path)>/*` — agreeing to a folder
once should not mean being asked again for the second file in it.

`shell` performs the analogous check itself via `command.cjs`'s `externalPaths()`
(§3.10a), because a shell command's paths are buried in argv rather than a single tool
parameter — same permission key, different detection mechanism.

### 5.2 Protected-record boundary: `refuseIfProtected`

A **third door** (beyond the app's own screens and `inertia_remove`'s protection check)
closing the same hole: an agent with ordinary file tools could reach
`agents/inertia-dev.json` via `write`/`edit`/`patch`/`file_move`/`file_delete` directly
and bypass Inertia's own protected-record logic. `refuseIfProtected(ctx, absPath, verb)`
(`outside.cjs:95-110`) fires for any path that is directly inside one of
`RECORD_FOLDERS = ["agents", "routines", "skills", "memory", "computers"]` and ends in
`.json`; reads and JSON-parses the file, checks `isProtected(record)` (from
`shared/agents.js`, not detailed in this doc), and throws (`name: "ProtectedRecord"`)
with the protection reason if so. Cheap for the ordinary case (only pays the read cost
for paths that actually match the record-folder shape).

### 5.3 The agent's own sandbox: `sandbox/` provider abstraction

`computer_*` tools never talk to Docker, Daytona, or the local filesystem directly —
every operation routes through `src/main/sandbox/index.cjs`, which resolves
`sandbox.forAgent(root, agentId)` to a `computers` collection record (a JSON file exactly
like every other workspace resource) and dispatches to one of three interchangeable
providers (`src/main/sandbox/provider.cjs` defines the contract; `providers/local.cjs`,
`providers/docker.cjs`, `providers/daytona.cjs` implement it):

```
id, label, blurb
available(config)                     -> { ok, reason, hint } — never a bare boolean
create(spec)                          -> the handle to persist
start / stop / remove(handle)
status(handle)                        -> one of provisioning|running|paused|stopped|error|missing
stats(handle)                         -> { cpuPct, memPct, diskPct } (each nullable, not zero-when-unknown)
exec(handle, request)                 -> { code, stdout, stderr, durationMs, timedOut, ok }
listDir / readFile / writeFile(handle, path)
snapshot / snapshots / restore(handle)
screenshot(handle)                    -> PNG data URL or null
```

Two invariants every provider must honor, directly relevant to a Rust port's trait
design: **(1)** nothing throws for a machine that's simply gone — `status` answers
`"missing"`, everything else degrades the same way a stopped machine would; **(2)** a
command's own exit code is **not** treated as a tool-layer error — `exec` resolves
(never rejects) for a failed command and rejects only when the command genuinely could
not be started, exactly mirroring the `shell` tool's own "let the model see the failure"
philosophy at the sandbox layer.

`sandbox.drive(root, computerId, action, args)` is the **single shared choke point** for
desktop actions (open/click/type/key/scroll/observe), used by *both* the `computer_*`
tools and the Browser-pane-on-sandbox UI, specifically to prevent the two callers from
independently reinventing (and silently diverging on) the browser-launch command — a
real bug class named in the source comments.

The **desktop command compiler** (`src/main/sandbox/desktop.cjs`, referenced but not
fully read for this doc) turns a list of `{kind: click|move|down|up|type|key|scroll|wait}`
primitives into one shell command batch executed via `sandbox.exec`, returning a
structured `OBSERVE_ANNOTATED` payload (screen size, active window, cursor position, a
base64 PNG, and — after a click — a base64 PNG close-up) parsed back by
`computer.cjs`'s `observationResult()`.

### 5.4 What is *not* sandboxed

`shell` (§3.10), the file tools (§3.1-3.9), `browser_*` (§3.17, drives the pane in the
actual app window with the actual user's cookies/localhost), `terminal_read` (§3.18), and
`lsp` (§3.19) all operate directly on the machine running Inertia — there is no sandbox
indirection for any of them. The `computer_*` family is the **only** builtin family that
routes through the sandbox provider abstraction; this asymmetry is deliberate and
explicitly documented in `computer.cjs`'s own header comment as the reason it exists as a
separate tool family rather than a flag on `shell`.

---

## 6. Rust design notes

### 6.1 `trait Tool` shape

The JS `defineTool` object maps almost directly onto a trait + struct pair:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn id(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> &JsonSchema;           // see 6.2
    fn permission(&self) -> &PermissionSpec;       // key, target_fn, always_fn
    fn render(&self, args: &Value) -> Option<String>;
    fn source(&self) -> ToolSource;                // Builtin | Mcp | OpenApi | Composio

    /// Mirrors `normalize`: repair a call shape before schema validation.
    /// Must never fail the call outright — on error, fall through to raw args
    /// and let validation produce the user-facing message.
    fn normalize(&self, raw: Value, ctx: &ToolCtx) -> Value { raw }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutcome, ToolError>;
}

pub struct PermissionSpec {
    pub key: String,
    pub target: Option<Box<dyn Fn(&Value) -> Option<String> + Send + Sync>>,
    pub always: Option<Box<dyn Fn(&Value) -> Option<String> + Send + Sync>>,
}

pub struct ToolOutcome {
    pub title: String,
    pub output: String,
    pub metadata: serde_json::Map<String, Value>,
    pub images: Vec<String>, // data: URLs, filtered non-empty on the way out
}
```

`target`/`always` as boxed closures over `&Value` mirrors the JS `(args) => string`
pattern exactly and keeps per-tool permission logic colocated with the tool definition
rather than centralized in a big match statement — the same locality the original design
values (each tool decides what a rule about it is *written about*; the session/permission
layer decides only *whether* it's granted).

`ToolError` should carry `retryable: bool` and an optional `refused_by_user: bool`
exactly as the JS version does — these two flags are what the session loop uses to
decide "end the turn" vs "let the model retry," and losing that distinction changes
observable behavior (a rule-based deny should let the model try something else; a human
"no" should end the tool-call batch for that turn).

### 6.2 Expressing JSON Schema

Do **not** reach for a general JSON Schema crate with `$ref`/`anyOf`/`oneOf` support —
the source is explicit that schemas are kept deliberately flat (`CONTRACT.md:38-40`:
"no `$ref`, no `anyOf`, no `additionalProperties: true`. Providers are strict and
inconsistent about all three"). A small closed enum matching exactly what `schema.cjs`
supports is the right port target:

```rust
pub enum Schema {
    String { description: String, enum_values: Option<Vec<String>> },
    Number { description: String, minimum: Option<f64> },
    Integer { description: String, minimum: Option<i64> },
    Boolean { description: String, default: Option<bool> },
    Array { description: String, items: Box<Schema> },
    Object { properties: BTreeMap<String, Schema>, required: Vec<String>, additional_properties: bool },
    Record { description: String }, // additionalProperties: true, no fixed shape
}
```

Builder functions (`s.str`, `s.int`, `s.object`, ...) map 1:1 to constructors/methods on
this enum. Serialize to the JSON-Schema wire format only at the point of building the
provider request — keep the Rust-side representation the source of truth, not a
`serde_json::Value` blob, so a tool's parameter contract is checkable at compile time
where reasonably possible.

**Validation** should port `schema.cjs`'s `validate()` behavior faithfully, including its
specific coercions (string<->number/bool leniency, scalar-to-single-item-array, absent
optional == explicit null unless required, `additionalProperties: false` silently drops
unknown keys rather than failing) — these are load-bearing for real-world model output,
not incidental. Error messages must remain **English sentences naming the field**, not
`serde`'s default path-based errors, since they are read by the model, not a developer.

### 6.3 Async execution and streaming partial output

`execute` is inherently `async` and must observe cancellation — the direct analogue is a
`tokio::select!` between the tool's own future and a `CancellationToken`/`oneshot`
cancel signal threaded through `ToolCtx`, mirroring `ctx.signal` (a Node `AbortSignal`).
Only cancellation should propagate as an `Err` that unwinds the call; every other failure
path (schema validation, permission denial, a thrown domain error) should resolve to an
`Ok(ToolOutcome)`-shaped-but-`ok:false` result, exactly as `runTool` does — this is a
deliberate asymmetry worth preserving precisely, not "simplifying" into uniform `Result`
semantics, because the session loop downstream depends on being able to distinguish
"the model should read this and react" from "the turn itself has ended."

For streaming partial output (`ctx.update`), an `mpsc::UnboundedSender<ToolUpdate>`
captured in `ToolCtx` is the natural fit:

```rust
pub struct ToolCtx {
    pub cwd: PathBuf,
    pub root: Option<PathBuf>,
    pub cancel: CancellationToken,
    pub session_id: String,
    pub message_id: String,
    pub call_id: String,
    pub agent: String,
    pub ask: Arc<dyn Fn(AskRequest) -> BoxFuture<'static, Result<AskDecision, PermissionDenied>> + Send + Sync>,
    pub update: mpsc::UnboundedSender<ToolUpdate>,
    pub tools: Arc<ToolRegistry>, // for composing tools (task/spawn)
    // ... session-specific fields (unattended, approval, depth, agent_id, etc.)
}
```

`shell`'s 200ms-interval live-tail pattern and `todowrite`'s "push immediately, resolve
later" pattern both map cleanly onto sending on this channel from within `execute` before
returning the final `ToolOutcome`.

### 6.4 Pluggable, mockable registry

Keep the same three-phase split the JS version has, since it's what makes the system
testable and extensible:

1. **A static builtin list** (`Vec<Arc<dyn Tool>>`, built once, analogous to the
   `registry.cjs:22-75` literal array) — order matters for prompt-prefix cache stability,
   so preserve insertion order exactly rather than re-sorting builtins.
2. **Dynamic sources** (MCP, OpenAPI, Composio — and, for testing, a `MockSource`)
   behind a `trait ToolSource { async fn tools(&self, root: &Path) -> Result<Vec<Arc<dyn
   Tool>>> }`, collected then **sorted by id** exactly as `fromSources()` does, and with
   the same "a broken source is dropped with a warning, never crashes the turn" behavior
   (`Result` per source, errors collected into a `problems: HashMap<String, String>` the
   caller can surface, not bubbled as a hard failure).
3. **Per-agent filtering** (`forAgent`) as a pure function over `(all_tools, rules,
   depth, agent) -> Vec<Arc<dyn Tool>>`, independent of how the tools were sourced — this
   is what makes injecting a `MockSource` producing a handful of fake tools with
   controllable `execute` closures trivial for integration tests of the permission-
   filtering and delegation-depth logic without needing a real MCP server or a real
   Composio account.

A `ToolRegistry` struct wrapping all three phases, with an explicit `fn builtins() ->
&[Arc<dyn Tool>]` and `fn all(root, include_task: bool) -> Vec<Arc<dyn Tool>>` and
`fn for_agent(root, rules, depth, agent) -> ToolSet` mirroring the exact three JS
functions (`builtins`, `all`, `forAgent`) keeps the port's test surface aligned with the
original's, and keeps the "which tools does agent X get right now" question answerable
by a single, unit-testable pure function exactly as it is today.

### 6.5 Things to port byte-for-byte rather than "improve"

A few subsystems are algorithmically subtle enough, and specifically tuned against
observed real model behavior, that a Rust rewrite should transliterate rather than
redesign:

- **`replace.cjs`'s match ladder** (§3.3a) — the rung order, the disproportionate-match
  guard's exact thresholds, and the "ambiguity is a hard failure, never a coin toss" rule
  are the entire reason `edit`/`patch` land reliably against real model output. A
  "smarter" fuzzy matcher risks silently editing the wrong span.
- **`command.cjs`'s `ARITY` table and `DANGEROUS`/`MASS_DELETE` pattern lists** (§3.10a)
  — these are curated from observed dangerous commands, not derived from a principle;
  port the literal lists, don't try to generalize them into a smaller rule set.
- **`truncate.cjs`'s JSON-aware shrinking** (§2.1) — the "clip strings before dropping
  array items" ordering and the "never send unparseable JSON" invariant are what keeps
  a truncated search result rendering as a card instead of a wall of text; a byte-offset
  `.slice()` truncation (the "obvious" simpler approach) actively regresses this.
- **The permission decision cascade order in §4.2** — the exact ordering of
  deny-check -> allow-check -> preapproved -> approval-dial -> hook -> unattended ->
  no-notifier -> aborted -> card is load-bearing: each step exists because of a specific
  observed failure mode (e.g. checking the hook *before* the unattended refusal so a
  routine with a permission-deciding hook doesn't uselessly refuse everything).
