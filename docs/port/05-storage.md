# 05 — Storage & Persistence Port Specification

Source of truth for this document: `D:\oss\inertia\src\main\session\` (`index.cjs`,
`ipc.cjs`, `store.cjs`, `compactions.cjs`, `session.test.js`), `src\main\workspace\`,
`src\main\project\`, `src\main\memory\`, `src\main\snapshot\`, plus
`src\renderer\lib\workspace-store.js` / `workspace-sync.js` (thread/message
persistence actually happens here, not in main — see §3) and `src\shared\*.js`.
File:line references point at the Electron source tree above unless stated
otherwise. Everything here describes the **current on-disk shape**; the Rust
rewrite must reproduce it byte-for-byte compatibly so an existing workspace
folder opens unchanged.

---

## 1. Complete on-disk tree

Two things persist outside any one workspace folder:

```
<Electron userData dir>/
  workspace.json                # { root, appVersion, updatedAt } — the ONLY pointer
                                 # to the workspace. Deleting it resets first-run
                                 # setup; the workspace folder itself is untouched.
```
(`src/main/workspace/root.cjs:15-17`). On Windows this is
`%APPDATA%\<AppName>\workspace.json`; on macOS `~/Library/Application
Support/<AppName>/workspace.json`; on Linux `~/.config/<AppName>/workspace.json`
(Electron's `app.getPath("userData")` convention — the Rust/Tauri port should
use the platform-equivalent `tauri::api::path::app_config_dir` or similar and
keep the filename `workspace.json`).

The **workspace folder** itself (chosen by the user on first run, default
suggestion `~/Documents/Inertia`, `root.cjs:54-62`) has this shape
(`layout.cjs:24-49`, confirmed against `scaffold.cjs` and `docs/workspace.md`):

```
<root>/
  inertia.json                        # manifest — marks folder as ours (scaffold.cjs:60-166)
  README.md                           # written once, human-readable explainer
  workspace.json                      # NOT here — see above, this is a different file

  settings/
    app.json                          # { version, workingDirectory?, seeded, seededAt, ... }
    appearance.json                   # { theme, zoomFactor, ... }
    models.json                       # { providers: [...], defaultModel, prices? }
    permissions.json                  # { workspace: [...rules], agents: { <agentId>: [...] } }
    identity.json                     # { user: {...} }
    group.json                        # { permissions: {...}, prompt? }
    projects.json                     # { folders: { <normalizedPath>: {...} } }
    voice.json                        # { ...voice/audio provider settings }
    computers.json                    # sandbox provider defaults (defaultProvider, specs, ...)

  agents/
    <id>.json                         # one file per agent record

  skills/
    <slug>/
      SKILL.md                        # YAML frontmatter + Markdown body
      (any other files the skill needs — scripts, references)

  plugins/
    mcp/<id>.json                     # MCP server definitions
    openapi/<id>.json                 # imported OpenAPI tool sources
    openapi/specs/<id>.json           # the raw OpenAPI spec document, kept beside its entry
    composio/<id>.json                # connected Composio apps/toolkits

  conversations/
    threads/<threadId>.json           # one thread (conversation) record
    messages/<threadId>.json          # { id: threadId, threadId, messages: [...] }

  history/
    executions/<id>.json              # routine/tool run records
    sessions/<turnId>.json            # one agent TURN as it ran (events, usage, status)
    activity/<YYYY-MM-DD>.json        # { date, events: [...] } — day-bucketed event log

  memory/
    <id>.json                         # one durable fact per file

  hooks/
    hooks.json                        # lifecycle hooks (Claude-Code-compatible shape)
    (hook scripts referenced by hooks.json live alongside, user-managed)

  routines/
    <id>.json                         # one scheduled/triggered playbook per file

  computers/
    <id>.json                         # one sandbox/machine record per file

  files/
    work/                             # default working directory when no project is chosen
    (attachments, generated output — otherwise unmanaged by the collection layer)

  secrets/
    secrets.json                      # { entries: { <NAME>: { value, label, createdAt, updatedAt } } }

  cache/
    composio-catalogue.json           # cached Composio app catalogue
    snapshots/<sha1-of-cwd-prefix16>/ # per-project shadow git object store (see §4)
      HEAD, objects/, refs/, info/exclude, ...
    compactions/<sha1-of-threadId-prefix24>.json   # per-thread compaction cache (see §6)

  logs/
    <YYYY-MM-DD>.log                  # app diagnostics, retained ~7 days
    failures/<YYYY-MM-DD>.jsonl       # one JSON line per failed tool/turn/hook/routine,
                                       # retained ~90 days (src/main/failures.cjs)

  backups/
    v<fromVersion>-<ISO-stamp-colons-as-dashes>/    # full pre-migration copy (migrate.cjs:78-91)
      (everything except backups/, logs/, .git/, .tmp/, node_modules/)
```

Notes on the tree:

- `plugins/mcp`, `plugins/openapi`, `plugins/openapi/specs`, `plugins/composio`,
  `conversations/threads`, `conversations/messages`, `history/executions`,
  `history/sessions`, `history/activity` are each declared as their own entry
  in `DIRECTORIES` (`layout.cjs:24-49`) purely so the settings-pane "what's in
  my folder" screen can list/size them individually; they are physically
  subdirectories of their parent as shown above.
- `cache/`, `logs/`, `backups/` are explicitly "safe to delete" — nothing
  reads them as the source of truth for a record; losing them loses
  performance/history, not data.
- A skill is the one collection whose record is a **folder**, not a single
  JSON file — `skills/<slug>/SKILL.md`, `kind: "skill"` in `COLLECTIONS`
  (`layout.cjs:64`). Anything else placed beside `SKILL.md` (scripts,
  reference docs) is preserved verbatim; the app never touches it.
- Directory names are load-bearing. `scaffold.cjs`'s README (`scaffold.cjs:23-45`)
  states this explicitly: "Do not rename the directories. They are the
  addresses the app resolves by name." The Rust port must keep every path
  string identical.

---

## 2. Storage primitives (the only place that touches disk)

`src/main/workspace/fsx.cjs` is, by design, **the only module that calls into
`fs`** for workspace data. Every other layer goes through it. The Rust port
should have exactly one crate/module with this responsibility.

### 2.1 Path confinement

`resolveInside(root, relPath)` (`fsx.cjs:20-27`): resolves `relPath` against
`root` and throws if the result is not `root` itself or does not start with
`root + path.sep`. No silent sanitizing of `..` — a path that escapes is a
hard error. **Rust**: `std::path::Path::canonicalize` (or manual resolution
without following symlinks, matching Node's `path.resolve` semantics which do
**not** touch the filesystem) then a prefix check.

### 2.2 Atomic writes: temp + rename + fsync

`writeThenRename(file, contents)` (`fsx.cjs:152-190`):

1. `ensureDir(dirname(file))`.
2. Write to `<file>.<pid>.tmp` via an open file handle.
3. `handle.writeFile(contents, "utf8")`.
4. `handle.sync()` (fsync) — **best-effort**, errors swallowed (network
   shares/virtual drives may reject `fsync` with `EINVAL`; the write itself
   already succeeded, so this is not fatal). This step exists specifically
   because rename-is-atomic only protects against a *visible* half-write, not
   against the OS reporting the new name exists while the bytes are still in
   page cache — the comment (`fsx.cjs:156-169`) documents a real data-loss
   incident (files full of NUL bytes after a power event) this fixed.
5. `handle.close()`.
6. Rename `tmp` → `file` via `whenReleased(...)` (see §2.3). On failure, the
   tmp file is removed (`fsp.rm(tmp, { force: true })`) and the error
   re-thrown.

`writeJson` = `writeThenRename(file, JSON.stringify(value, null, 2) + "\n")`
(pretty-printed, 2-space indent, trailing newline). `writeText` is the same
without JSON encoding.

**Rust**: write to a sibling temp file (`format!("{file}.{pid}.tmp")`), flush
+ `sync_all()` (best-effort, ignore platform errors like `EINVAL`), close,
then `std::fs::rename` wrapped in the retry below. Use `serde_json::to_string_pretty`
+ trailing `\n` to match byte-for-byte formatting (matters for anyone diffing
the folder in git, which is an explicitly supported workflow).

### 2.3 Windows retry-on-contention

`whenReleased(operation)` (`fsx.cjs:120-132`): retries an operation (used only
for rename and remove) when it fails with `EBUSY`, `EPERM`, or `EACCES`, with
delays `[20, 60, 150, 400, 800, 1200]` ms (six attempts, ~2.6s total), then
gives up and rethrows. This exists because antivirus/search-indexer/backup
agents transiently hold Windows file handles open; without the retry, renames
silently failed, leaving orphaned `.tmp` files and — worse — leaving deletes
"succeeding" from the app's point of view while the file reappeared on next
list (`fsx.cjs:102-119` documents an incident where a deleted conversation
"came back"). **Rust**: replicate the same error-code set and delay ladder
(`ERROR_SHARING_VIOLATION`/`ERROR_ACCESS_DENIED` map to these `errno`s via
Node; on Windows Rust's `std::io::Error::raw_os_error()` gives the Win32 code
directly — treat `ERROR_SHARING_VIOLATION` (32), `ERROR_LOCK_VIOLATION` (33),
and `ERROR_ACCESS_DENIED` (5) the same way).

### 2.4 Corruption handling — never silently treat damage as empty

`readJson(file, fallback)` (`fsx.cjs:67-81`): if the file exists but
`JSON.parse` fails, the raw bytes are copied once to `<file>.damaged`
(`keepDamaged`, `fsx.cjs:90-100`, written only if that sidecar doesn't already
exist — first copy nearest the failure wins) and **then** `fallback` is
returned. Read failures for "file not found" return `fallback` directly with
no damaged-copy (that's the ordinary case, not corruption). This distinction
matters: naively treating "empty/unparsable" the same as "absent" was a real
bug that silently destroyed a conversation (comment at `fsx.cjs:51-66`) — a
thread whose JSON failed to parse was read as empty, rendered empty, and the
next save overwrote the only copy with that emptiness.

**Rust**: `readJson<T>(path) -> Result<T, ReadError>` where `ReadError`
distinguishes `NotFound` (return default, no side-effect) from `ParseError`
(copy raw bytes to `<file>.damaged` if not already present, then return
default, and — importantly — surface a warning the caller can propagate to
the user; the JS layer does not currently surface it beyond the sidecar file,
which is arguably a gap the Rust rewrite could close by logging).

### 2.5 Directory listing

`listDir(dir, { onlyDirs, ext })` (`fsx.cjs:201-214`): lists entries, filters
dotfiles, `*.tmp`, and `*.damaged`, optionally by extension, returns names
**sorted** with `localeCompare`. Missing directory → `[]`, not an error.

### 2.6 Slugs / ids

`slugify(value, fallback)` (`fsx.cjs:244-252`): lowercase, NFKD-normalize,
collapse non-`[a-z0-9]` runs to `-`, trim leading/trailing `-`, truncate to 64
chars, fallback to `fallback` (e.g. `"agent"`, `"memory"`) if empty.

---

## 3. The collection/document CRUD layer

`src/main/workspace/collections.cjs` is the single CRUD surface every
collection and document goes through — **the only place that knows the codec
per collection kind**. Adding a resource type is a line in `layout.cjs`'s
`COLLECTIONS`/`DOCUMENTS` maps, not new code here.

### 3.1 Collections (`COLLECTIONS` map, `layout.cjs:60-74`)

```js
{
  "plugins.mcp":      { dir: "plugins/mcp",      kind: "json",  singular: "mcp" },
  "plugins.openapi":  { dir: "plugins/openapi",  kind: "json",  singular: "openapi" },
  "plugins.composio": { dir: "plugins/composio", kind: "json",  singular: "composio" },
  skills:             { dir: "skills",           kind: "skill", singular: "skill" },
  agents:             { dir: "agents",           kind: "json",  singular: "agent" },
  routines:           { dir: "routines",         kind: "json",  singular: "routine" },
  computers:          { dir: "computers",        kind: "json",  singular: "computer" },
  memory:             { dir: "memory",           kind: "json",  singular: "memory" },
  threads:            { dir: "conversations/threads",  kind: "json", singular: "thread" },
  messages:           { dir: "conversations/messages", kind: "json", singular: "messages" },
  executions:         { dir: "history/executions", kind: "json", singular: "run" },
  sessions:           { dir: "history/sessions",   kind: "json", singular: "turn" },
  activity:           { dir: "history/activity",   kind: "json", singular: "event" },
}
```

Two codecs today: `json` (one `<id>.json` file) and `skill` (one
`<id>/SKILL.md` file with YAML frontmatter). Adding SQLite as a third `kind`
is architecturally trivial at this layer (see §7.3) but changes the
portability contract (§7.3 discusses the tradeoff).

**Operations** (`collections.cjs`):

- `ids(root, name)` — lists filenames (sans `.json`) or subdirectory names for
  `skill` kind, via `fsx.listDir`.
- `get(root, name, id)` — reads one record. For `json`, `{...record, id}` (id
  is always re-derived from the filename, never trusted from file content).
  For `skill`, reads `SKILL.md`; if the folder exists but has no `SKILL.md`,
  returns a "missing" record (`disabled: true`, a `problems` array) rather
  than `null` — a silently-nonfunctioning skill must still show up.
- `list(root, name)` — maps `ids` through `get`, and **catches per-record
  errors** (`.catch(() => null)`, then filters falsy) so one corrupt file
  never blanks the whole list.
- `put(root, name, input)` (`collections.cjs:112-138`) — create-or-replace.
  Id: use `input.id` if present (validated via `assertId`: must be a single
  path segment, no `.`/`..`/separators/absolute paths/NUL); otherwise derive a
  new unique id from `input.name ?? input.title` via `uniqueId` (slugify,
  then suffix `-2`, `-3`, ... up to 500 attempts, then a millisecond-timestamp
  suffix as last resort). Stamps `createdAt` (preserved from the existing
  record if any, else now) and `updatedAt` (always now) — **the caller never
  controls these two fields directly**, everything else in `input` is written
  through verbatim.
- `patch(root, name, id, changes)` — shallow-merges `changes` onto the
  existing record via `get` then `put`; throws if the record doesn't exist
  (patch is never an upsert).
- `remove(root, name, id)` — deletes the file (or folder, for `skill`).
- `rename(root, name, id, nextId)` — write under `nextId` (fails if it already
  exists), then remove `id`.

**Rust design**: a `CollectionStore` trait —

```rust
#[async_trait]
trait CollectionStore {
    async fn ids(&self, name: &str) -> Result<Vec<String>>;
    async fn get(&self, name: &str, id: &str) -> Result<Option<Value>>; // or generic <T>
    async fn list(&self, name: &str) -> Result<Vec<Value>>;
    async fn put(&self, name: &str, record: Value) -> Result<Value>;
    async fn patch(&self, name: &str, id: &str, changes: Value) -> Result<Value>;
    async fn remove(&self, name: &str, id: &str) -> Result<()>;
    async fn rename(&self, name: &str, id: &str, next_id: &str) -> Result<Value>;
}
```

with one implementation backed by the JSON-files codec (matching the current
behavior exactly) and, if SQLite is adopted for high-churn collections (see
§7.3), a second implementation behind the same trait — callers (session
loop, IPC handlers) never need to know which is active. Prefer typed
per-collection wrappers (`ThreadStore`, `AgentStore`, ...) built over a
generic `JsonFileCollection<T: Serialize + DeserializeOwned>` rather than
threading `serde_json::Value` everywhere; the id-stamping/timestamp logic
belongs in that generic layer so every collection gets it for free, exactly
as `collections.cjs` does today.

### 3.2 Documents (`DOCUMENTS` map, `layout.cjs:77-103`)

Single JSON files, not collections — read/written as whole objects via
`readDocument(root, key, fallback)` / `writeDocument(root, key, value)`. No
id, no timestamps, no merge semantics (`ws:doc:set` — `workspace/index.cjs:337-341`
— replaces the file's parsed value wholesale; callers do their own
read-modify-write when they only want to touch one field, e.g.
`syncDocument` in the renderer, §3.4).

```
"settings.app"          -> settings/app.json
"settings.appearance"   -> settings/appearance.json
"settings.models"       -> settings/models.json
"settings.permissions"  -> settings/permissions.json
"settings.identity"     -> settings/identity.json
"settings.group"        -> settings/group.json
"settings.projects"     -> settings/projects.json
"settings.voice"        -> settings/voice.json
"settings.computers"    -> settings/computers.json
"secrets"               -> secrets/secrets.json
"hooks"                 -> hooks/hooks.json
"cache.composio"        -> cache/composio-catalogue.json
```

`documentKeyForPath` (`layout.cjs:114-118`) is the reverse map, used by the
filesystem watcher to turn a changed path back into a document key
(normalizes both `\` and `/`).

### 3.3 The folder watcher

`src/main/workspace/watch.cjs` watches the whole workspace (`fs.watch` with
`recursive: true` where available) and, for any change **not** already
announced by an IPC handler's own broadcast, maps the changed path to either
a document key or a collection name and re-broadcasts `workspace:changed` to
the renderer. This is what lets an agent's `write` tool creating
`agents/x.json` directly (bypassing the collections API) still show up live
in the UI. **Rust/Tauri equivalent**: `notify` crate watching the root
recursively, mapped through the same path tables, emitting a Tauri event.

### 3.4 Where threads/messages are *actually* written — renderer-owned

Critically: despite `threads`/`messages` being declared as ordinary
collections in `layout.cjs`, the **live conversation state is owned by the
renderer's React store**, not by the main process. `src/main/session/index.cjs`
(the agent loop) receives a `history` array from the caller, runs the turn,
and returns the updated transcript — it never writes `conversations/*`
itself. The only main-process writers of `threads`/`messages` are the
routines scheduler (`src/main/routines/index.cjs:292-353`, for a routine's
own synthetic conversation) and the demo-data seeding path.

The renderer's persistence hook, `useWorkspacePersistence`
(`src/renderer/lib/workspace-store.js:225-437`), is the real source of the
on-disk thread/message shape:

- **Hydration** (first open): if `settings/app.json.seeded` is true, read
  every collection + document and apply it into React state. Otherwise (first
  ever open of this folder), **seed the folder from whatever is currently in
  memory** (demo data) and set `seeded: true` — a workspace is never an empty
  shell on first run.
- **Mirroring** (every change): a `SOURCES` table (`workspace-store.js:88-173`)
  drives a 400ms-debounced diff-and-write (`syncCollection`,
  `workspace-sync.js:300-354`): each record is hashed (`fingerprint`, a
  double 32-bit FNV-1a walk plus length — chosen over `JSON.stringify` caching
  specifically to avoid holding a second full copy of every conversation in
  memory, `workspace-sync.js:20-47`); only records whose hash changed since
  last write are sent to `client.put`; records that vanished are `client.remove`d
  **unless** a per-source `keep` predicate vouches for them (threads: kept
  if their message array still has content — a thread record is never
  deleted while its messages still exist in memory, only an explicit user
  delete removes both together).
- **Live re-read**: on `workspace:changed` (folder watcher or another
  window's write), only the named collection is re-listed and merged in —
  never a full re-hydrate, and the "already-seen-this-object" hash shortcut
  is dropped for that collection so external edits aren't missed.

A **draft** thread (`draft: true` — created by "New chat" before any message
is sent) and a **temporary** thread (`temporary: true` — explicitly marked
"don't save") are filtered out of what's ever written (`workspace-store.js:98`
— `SOURCES[0].read`), so clicking New chat and not sending anything leaves no
file behind. This is a policy decision the Rust port must replicate at
whichever layer owns thread lifecycle (likely the Tauri backend, if the port
moves persistence server-side — see §7.3 recommendation on where the
authoritative write path should live).

**Rust port implication**: the cleanest design is to move this
diff-and-persist responsibility into the Rust backend (it does not need to
stay renderer-owned; Electron's split existed because the renderer already
held the live React state and Node's IPC round-trip made pushing every
keystroke to main wasteful). A Tauri command surface mirroring `ws:put` /
`ws:remove` / `ws:doc:set`, called from the frontend on the same debounce, is
a faithful and simpler port of this behavior — see §7.2.

---

## 4. Record schemas

All records get `id`, `createdAt`, `updatedAt` stamped by `collections.put`
(§3.1) except documents (§3.2), which have no such envelope.

### 4.1 `settings/app.json` (document `settings.app`)

```json
{ "version": 1, "workingDirectory": null, "seeded": true, "seededAt": "2026-01-01T00:00:00.000Z" }
```
Seeded at scaffold with `{ version: WORKSPACE_VERSION }` (`scaffold.cjs:49`).
`workingDirectory`, if set, overrides the default `files/work` fallback used
when an agent has no `cwd` of its own (`session/ipc.cjs:89-95`).

### 4.2 `settings/appearance.json` (`settings.appearance`)

```json
{ "theme": "system" }
```
Seeded default (`scaffold.cjs:51`). Additional fields observed in the wild
(`docs/02-SETUP-AND-CONFIG.md:552`): `zoomFactor` (0.7–1.5).

### 4.3 `settings/models.json` (`settings.models`)

```jsonc
{
  "providers": [
    {
      "id": "anthropic", "name": "Anthropic", "enabled": true,
      "baseUrl": "https://api.anthropic.com/v1",
      "apiKeySecret": "ANTHROPIC_API_KEY",   // name of an entry in secrets.json, never the key itself
      "kind": "anthropic",                    // or "openai"; omitted = inferred from baseUrl
      "headers": {},
      "models": [{ "id": "claude-sonnet-4-5", "label": "", "context": 200000 }]
    }
  ],
  "defaultModel": "anthropic/claude-sonnet-4-5",
  "prices": {                                 // optional override map, keyed by model id
    "anthropic/claude-sonnet-4-5": { "input": 3, "output": 15 }
  }
}
```
Read via `storedProviders()` (`src/main/llm/index.cjs:77-82`):
`{ providers: doc.providers ?? [], defaultModel: doc.defaultModel ?? "" }`.
Seeded empty: `{ providers: [] }` (`scaffold.cjs:51`). `kind` decides wire
protocol (Anthropic-native vs OpenAI-chat-completions); getting this wrong
silently loses prompt caching.

### 4.4 `settings/permissions.json` (`settings.permissions`)

```jsonc
{
  "workspace": [                    // array of rules, applied over shared defaults
    { "tool": "shell", "pattern": "*", "action": "ask" }
  ],
  "agents": {                       // per-agent overrides, keyed by agent id
    "atlas": [ { "tool": "edit", "pattern": "*.env", "action": "deny" } ]
  }
}
```
Seeded `{ rules: [] }` at scaffold time — note this is the *old* shape; the
live code path (`session/index.cjs:456-471`, `rulesFor`) reads `workspace`
(array) and `agents.<id>` (array), and explicitly **ignores** the legacy
`rules` map-of-capability-flags shape written by older builds (rather than
mis-translating it) — ignored means "fall back to defaults", not an error.
Rule shape: `{ tool, pattern, action }` where `action` is one of the
permission-decision enum (`allow` / `ask` / `deny` — see
`src/shared/permission.js`). Layering order for effective permission
(`rulesFor`, `session/index.cjs:456-471`): built-in `defaultRules()` (one
`{tool, pattern:"*", action: tool.fallback}` per non-dynamic tool,
`src/shared/tools.js:401-407`) → workspace rules → per-agent rules → the
agent record's own inline `permission` array (`agent.permission`), each
layer able to override the one before it (`sharedPermission.merge(...)`).

### 4.5 `settings/identity.json` (`settings.identity`)

```jsonc
{
  "user": {
    "name": "Ana",
    "email": "",
    "avatar": "settings/avatar.png",     // relative path, read via ws:file:read-image
    "preferences": {
      "defaultAgentId": "atlas",
      "defaultMode": "autonomous",        // chat | plan | autonomous
      "defaultApproval": "ask",           // ask | edits | auto
      "sendOnEnter": true,
      "toolAccess": "all",                // "all" | "on-demand" (tools/groups.cjs gate)
      "memory": true,
      "memoryBackend": "local",           // "local" | an MCP server id
      "memoryCapture": "session",         // "session" | "turn" | "off"
      "memoryReview": false,
      "memoryBudget": 2560,
      "memoryProject": true,
      "memoryPerAgent": false,
      "memoryInstructions": ""
    }
  }
}
```
Seeded `{}` (scaffold.cjs:53). Read fresh every turn (never cached) so an
edit made in the user's own editor takes effect on the next message
(`session/index.cjs:586-591`). `settingsOf(profile)` (`memory/index.cjs:39-55`)
is the canonical reader of the memory-related preference subset.

### 4.6 `settings/group.json` (`settings.group`)

```jsonc
{
  "permissions": { "canInvite": true, "canHandover": true, "canLeave": true, "maxAgents": 6 },
  "prompt": null      // optional custom instructions injected into the room's system prompt
}
```
Governs multi-agent "room" conversations (`src/main/team/group.cjs`). Not
persisted per-conversation roster state (that's in-memory, `group.cjs:69-76`
— "This table lives in memory" — a room is reconstructed by re-seating from
the thread's agent list on every turn, `session/ipc.cjs:521-569`).

### 4.7 `settings/projects.json` (`settings.projects`)

```jsonc
{
  "folders": {
    "d:/work/api": { "createdAt": "2026-01-01T00:00:00.000Z", "declined": false }
  }
}
```
Key = `keyFor(folder)` (`project/index.cjs:48-53`): backslashes normalized to
forward slashes, trailing slash trimmed, lowercased — stable across
drive-letter case and separator style. Tracks whether an `AGENTS.md` /
`.inertia/rules/` starter was created or declined for a project folder, kept
in the *workspace* rather than in the project itself so a repository never
gains a file recording that Inertia once asked about it.

### 4.8 `settings/voice.json` (`settings.voice`)

Seeded `{}` (scaffold.cjs:56) — nothing is spoken until a key is stored, so
starts empty rather than a switch that repeats that absence. Holds
voice/TTS/STT provider configuration; exact field set is provider-specific
(see `src/renderer/lib/voice.js`, `src/renderer/features/settings/panes/VoicePane.jsx`) —
out of scope for message/session persistence, treat as an opaque settings
blob for the port's initial pass.

### 4.9 `settings/computers.json` (`settings.computers`)

```jsonc
{
  "defaultProvider": "docker",          // "local" | "docker" | "daytona"
  "daytonaApiUrl": "",
  "daytonaImage": "",
  "autoStopMinutes": 30,
  "defaultSpecs": { "cpu": 2, "memoryGb": 4, "diskGb": 20 },
  "pollSeconds": 10
}
```
Defaults + merge logic in `src/main/sandbox/index.cjs:49-93`
(`SETTINGS_DEFAULTS`, `settings()`, `saveSettings()`).

### 4.10 `secrets/secrets.json` (`secrets`)

```jsonc
{
  "entries": {
    "ANTHROPIC_API_KEY": {
      "value": "sk-ant-...",           // stored IN THE CLEAR — deliberate, documented limitation
      "label": "",
      "createdAt": "2026-01-01T00:00:00.000Z",
      "updatedAt": "2026-01-01T00:00:00.000Z"
    }
  }
}
```
`src/main/workspace/secrets.cjs`. Name must match `/^[A-Za-z_][A-Za-z0-9_]*$/`.
`{secret:NAME}` is substituted at use time anywhere in a config value
(recursively through strings/arrays/objects, `secrets.cjs:88-103`) — so an
MCP server's `env`/`headers`, an OpenAPI auth block, etc. reference secrets by
name and the raw value never round-trips back into those files. **Rust
port**: reproduce the plaintext storage as-is for parity (the app explicitly
says "for now" — do not silently "improve" this into encrypted storage
without the same UI disclosure, or a workspace shared between an old and new
build will disagree about the format).

### 4.11 `agents/<id>.json`

Fields observed across the codebase: `id`, `name`, `description?`, `model`
(a `provider/model` ref string), `temperature?`, `maxTokens?`,
`permission?` (array of `{tool,pattern,action}`, layered on top of workspace
rules — §4.4), `cwd?` (working directory override), `computerId?` (assigned
sandbox, written by `sandbox/index.cjs:568-588` `assign`), `protected?`
(true for the bundled "Inertia Dev" agent — blocks edit/delete everywhere,
`docs/workspace.md:92-99`), `paused?`/`pausedReason?` (`shared/agents.js`),
`tags?`, `stats: { messages, routinesRun, tokensUsed }`, `lastActiveAt?`,
`createdAt`, `updatedAt`. A legacy `capabilities` array field is dropped on
load if present (`workspace-sync.js:227-263`, `settleAgent`) — it named no
real tool and nothing ever read it.

### 4.12 `plugins/mcp/<id>.json`, `plugins/openapi/<id>.json`, `plugins/composio/<id>.json`

See `docs/workspace.md:105-169` (verified current — matches `plugins/*`
loader code):

```jsonc
// MCP
{
  "id": "playwright-mcp", "name": "Playwright MCP", "enabled": true,
  "type": "stdio",                                  // or "http"
  "command": "npx", "args": ["-y", "@playwright/mcp@latest"],
  "cwd": "", "env": { "KEY": "{secret:MY_TOKEN}" },
  "url": "https://mcp.example.com/mcp",              // when type = "http"
  "headers": { "Authorization": "Bearer {secret:MY_TOKEN}" },
  "timeoutMs": 0
}
// OpenAPI
{
  "id": "statuspage", "name": "Statuspage", "enabled": true,
  "specUrl": "https://api.statuspage.io/v1/openapi.json",
  "specPath": "plugins/openapi/specs/statuspage.json",
  "baseUrl": "", "auth": { "type": "bearer", "secret": "STATUSPAGE_TOKEN" }
}
// Composio
{
  "id": "github", "name": "GitHub", "toolkit": "github", "enabled": true,
  "authMode": "oauth", "apiKeySecret": "COMPOSIO_API_KEY",
  "entityId": "default", "connectionId": "", "connected": false, "tools": []
}
```

### 4.13 `skills/<slug>/SKILL.md`

Frontmatter + Markdown body (`src/main/workspace/skills.cjs`,
`frontmatter.cjs`). Known frontmatter keys: `name`, `description`, `enabled`
(default true), `tags` (array), `version`, `createdAt`, `updatedAt`. Any
other frontmatter key is round-tripped through an `extra` bag on the record
and re-serialized verbatim — the app must never silently drop a hand-written
key like `allowed-tools:`.

```markdown
---
name: Code review
description: Read a diff and say what is wrong
enabled: true
tags: [dev, qa]
---

# Review

Read the diff.
```
A folder with no `SKILL.md` still produces a record — `missingRecord(id)`
(`skills.cjs:72-87`) — disabled, with a `problems` array, rather than being
silently invisible.

### 4.14 `routines/<id>.json`

One scheduled/triggered playbook per file. Carries its own `mode` (default
`"autonomous"`) and `approval` (default `"auto"`) dials — unattended, "ask"
means "refuse" (nobody is there to answer), so the default is looser than a
conversation's — plus `runHistory` (append-only, most-recent-run style; see
`docs/workspace.md:253-263`: "A routine carries just its most recent run, so
execution history is append-only... diffing it the way threads are diffed
would read 'last hour's run is no longer in the list' as 'delete it'"). Full
detail in `src/main/routines/index.cjs`; out of primary scope for this
document but the storage pattern (append-only field on an otherwise
diff-synced record) is worth preserving exactly.

### 4.15 `computers/<id>.json`

```jsonc
{
  "id": "cmp_abc", "name": "New computer", "provider": "docker",
  "status": "running",                 // provisioning | running | stopped | error | missing
  "handle": "container-id-or-sandbox-id-or-path",   // provider-opaque; ONLY the provider interprets it
  "machineName": null, "os": "", "image": null, "workdir": "/workspace",
  "specs": { "cpu": 2, "memoryGb": 4, "diskGb": 20 },
  "usage": { "cpuPct": 0, "memPct": 0, "diskPct": 0, "scope": null, "at": "..." },
  "assignedAgentIds": ["atlas"],
  "snapshotCount": 0, "lastUsedAt": "...", "startedAt": null, "error": null
}
```
`src/main/sandbox/index.cjs:210-282` (`create`), `355-361` (`refresh`),
`568-588` (`assign`). The record is written **twice** during creation —
`status: "provisioning"` before the provider does anything, then patched with
the real `handle`/`status` — specifically so a crash mid-provision never
leaves an unrecorded container.

### 4.16 `memory/<id>.json`

```jsonc
{
  "id": "mem_01...",
  "title": "Prefers TypeScript strict mode",
  "body": "...",
  "description": null,                 // optional one-liner override for the prompt line
  "kind": "preference",                 // fact | preference | contact | project | credential-note | handover
  "scope": "global",                    // global | project
  "folder": null,                       // required when scope = "project"; absent = treated as global
  "tags": ["typescript"],
  "source": "learned",                  // learned | pinned | imported | agent
  "pinned": false,
  "pending": false,                     // true = awaiting user review, not yet used in prompts
  "agentId": null,                      // set when memoryPerAgent scoping is on
  "useCount": 0,
  "lastUsedAt": "2026-01-01T00:00:00.000Z",
  "createdAt": "...", "updatedAt": "..."
}
```
`src/shared/memory.js` is the pure logic (BM25-style scoring, dedup,
merge — see §6). One fact per record, deliberately: "a record holding three
things cannot be superseded when one of them changes" (`memory.js:1-8`). A
`kind: "handover"` record is special: one per project folder, rewritten (not
appended) each capture pass (`memory/capture.cjs:230-257`), `pinned: true`
always, title `"Where we left off in <basename(folder)>"`.

### 4.17 `hooks/hooks.json` (`hooks`)

Claude-Code-compatible shape deliberately, so scripts written for Claude Code
run unmodified. See `src/main/hooks/index.cjs` and `docs/hooks.md` (not
re-derived here — out of this document's scope beyond noting the storage
location and compatibility goal).

### 4.18 `cache/composio-catalogue.json` (`cache.composio`)

Opaque cached payload of Composio's app catalogue; safe to delete, rebuilt on
demand.

---

## 5. Conversation storage in depth

### 5.1 Thread record (`conversations/threads/<id>.json`)

Constructed in `createThread` (`src/renderer/lib/store.jsx:429-472`):

```jsonc
{
  "id": "thr_a1b2c3",              // uid("thr") — renderer-generated, NOT a filename-derived id here;
                                     // collections.put still accepts caller ids verbatim (assertId)
  "agentId": "atlas",
  "title": "New conversation",      // replaced by an LLM-generated title after the first message
  "mode": "autonomous",             // chat | plan | autonomous — user default, or per-thread override
  "approval": "ask",                // ask | edits | auto
  "draft": false,                   // true until the first message is sent; never persisted while true
  "pinned": false,
  "unread": 0,
  "updatedAt": "2026-01-01T00:00:00.000Z",
  "preview": "first 120 chars of the last message or attachment name",
  "messageCount": 3,
  "computerAttached": null,         // agent's assigned sandbox at thread-creation time
  "createdAt": "...",               // stamped by collections.put on first real write
  "temporary": false                // never persisted while true (see §3.4)
}
```
Sorted `byUpdatedDesc` on read (`workspace-sync.js:50-51`).

### 5.2 Message storage: one file per thread holding the WHOLE array

`conversations/messages/<threadId>.json`:

```jsonc
{ "id": "thr_a1b2c3", "threadId": "thr_a1b2c3", "messages": [ /* full array, newest last */ ] }
```

This is **not an append log** — it is a single JSON document rewritten in
full on every debounced sync (`asMessageRecords`, `workspace-sync.js:72-84`).
The array order is send/receive order (oldest first); the UI renders it
top-to-bottom as-is.

**Message record shape** (`store.jsx:1093-1178`, plus streaming fields):

```jsonc
// user message
{
  "id": "msg_x1",
  "role": "user",
  "content": "fix the login bug",
  "createdAt": "2026-01-01T00:00:00.000Z",
  "status": "sent",                 // sent | streaming | error
  "attachments": [ /* optional, only present if non-empty */ ]
}
// agent reply (as constructed at send time, before streaming)
{
  "id": "msg_x2",
  "role": "agent",
  "agentId": "atlas",
  "content": "",
  "createdAt": "...",
  "status": "streaming",
  "model": "anthropic/claude-sonnet-4-5",
  "parts": [ /* filled in as the turn streams — text/tool-call parts */ ],
  "stopped": false,                 // set true if interrupted
  "error": null,
  "thinking": [ /* signed reasoning blocks, current-turn only per Anthropic's API contract */ ]
}
```
`parts` entries include at least `{type:"text", text}` and
`{type:"tool", state: "running"|"done"|"failed", output, ...}` — a tool
part's `state` is what the composer's Stop/Send toggle and the "is this
conversation busy" check key off.

**Reconciliation on load** (`reconcileMessages`, `workspace-sync.js:159-186`):
a message read back from disk with `status: "streaming"` is *not* necessarily
abandoned — if this window is still holding a message with that id in memory,
memory wins (disk is behind by construction, written on a debounce). Only a
`streaming` message this window has **never heard of** is settled to
`error`/`sent` via `settleOnLoad` (`workspace-sync.js:109-138`): partial text
is kept, `stopped: true` is set, any `running` tool part becomes `failed`
with `"This call was interrupted and never finished."`. **This settling rule
is essential for the Rust port**: a process crash or force-quit must never
leave a message permanently stuck showing "Stop" in the composer.

A message that exists in memory but hasn't reached disk yet (still inside the
400ms debounce window) is **kept**, appended after whatever disk returned —
never dropped (`workspace-sync.js:172-185`).

### 5.3 Ordering / ids

- Thread ids: `uid("thr")` (renderer-side unique-id generator).
- Message ids: `uid("msg")`.
- Turn ids (§5.4): `` `turn-${Date.now().toString(36)}-${++nextId}` `` — unique
  across restarts because it's time-seeded, not because the in-process
  counter is durable (`session/ipc.cjs:307`).
- No secondary index anywhere. Listing a collection means `readdir` + parse
  every file (§3.1 `list`). At the workspace scale this targets (hundreds of
  threads, not millions) this is fine; see §7.3 for when that stops being
  true.

### 5.4 Turns (agent runs) — `history/sessions/<turnId>.json`

Distinct from a *message*: a **turn** is one full agent-loop execution (one
user message in, one or more tool calls, one final reply out), recorded by
`src/main/session/store.cjs` — **this is the layer that survives a window
reload**, independent of the renderer's own thread/message persistence.

```jsonc
{
  "id": "turn-abc123-7",
  "threadId": "thr_a1b2c3",
  "messageId": "msg_x2",
  "agentId": "atlas", "agentName": "Atlas",
  "provider": "anthropic", "model": "claude-sonnet-4-5", "modelRef": "anthropic/claude-sonnet-4-5",
  "cwd": "d:/work/api",
  "startedAt": "2026-01-01T00:00:00.000Z",
  "endedAt": "2026-01-01T00:00:07.000Z",
  "status": "complete",             // running | complete | error | cancelled | interrupted
  "steps": 4,
  "usage": { "input": 12000, "output": 800, "cacheRead": 0, "cacheWrite": 0 },
  "events": [ /* see REPLAYABLE set below; deltas merged, not one-event-per-token */ ],
  "truncated": false                // true once events.length exceeded MAX_EVENTS and the head was dropped
}
```

Key mechanics (`session/store.cjs`):

- **Write cadence**: debounced 500ms (`WRITE_DELAY`) while `status ===
  "running"`; flushed immediately (no debounce) the instant the turn ends.
  `root` is stripped from the persisted object (it's routing metadata, not
  part of the record).
- **Event merging**: only event types in `REPLAYABLE` (`delta`, `changes`,
  `reasoning`, `tool-start`, `tool-end`, `steer`, `warning`, `hook`, `error`,
  `done`) are stored at all — `tool-update` progress pings are not, since the
  following `tool-end` supersedes them entirely. Consecutive `delta`/`reasoning`
  events are **concatenated into the previous event** rather than appended
  separately — a long reply is thousands of stream chunks and storing each
  one would bloat the file to describe one page of text.
- **Bounded growth**: `MAX_EVENTS = 4000` — once exceeded, the *oldest*
  events are dropped (`truncated: true` set) because what matters for
  rejoining a live turn is the tail, not the whole history.
- **Screenshot thinning**: any event whose `metadata.screenshot` or
  `metadata.closeup` is set counts as "carrying a picture"; only the last
  `KEEP_RECORDED_FRAMES = 3` such events in **this stored record** keep their
  image bytes — earlier ones have `screenshot`/`closeup` stripped from
  `metadata` (`forgetOldFrames`). This is a *storage-layer* thinning,
  independent of the separate in-conversation screenshot thinning done inside
  the agent loop itself (`KEEP_SCREENSHOTS = 2`, `session/index.cjs:227`) —
  both exist because a driven-browser turn can otherwise write ~250MB across
  its debounced saves.
- **In-memory retention cap**: up to `KEEP_FINISHED = 20` *finished* turns per
  process keep their full `events` array in memory; older finished turns have
  `events` cleared and `shed: true` set (never re-fetched from memory again —
  `find(id)` falls back to re-reading the JSON file, which is the durable
  copy). **Running turns are never shed**, regardless of count.
- **Startup reconciliation**: `settleWorkspace(root)` scans every stored
  session with `status === "running"` (impossible — no process is running
  it) and rewrites it to `status: "interrupted"`, `endedAt: endedAt ??
  startedAt`. Same idea as the message-level settling in §5.2, applied to the
  turn-record layer.
- **Per-conversation cleanup**: `forget(threadId)` drops **finished** in-memory
  records for a thread (never touches the on-disk copies or running turns) —
  called from `agent:forget` when a conversation is deleted, alongside
  clearing permission grants, todo state, task/subagent bookkeeping, and
  read-state tracking for that thread.

`agent:history` (`session/ipc.cjs:624-633`) lists every turn for a thread by
reading the whole `sessions` collection and filtering — again, no index; a
linear scan of the folder.

### 5.5 Title generation

`src/shared/title.js`: after the **first** message in a thread, an
LLM call (temperature 0, max 256 tokens, no tools) is fired — not awaited —
with `TITLE_INSTRUCTION` as system prompt and `titlePrompt(text)` (first 1200
chars of the message, fenced as `<conversation>...</conversation>` material
to *name*, not an instruction to follow — defends against prompt injection
via the first message) as the user turn. The raw reply is cleaned
(`cleanTitle`): strip `<think>` blocks and any HTML-ish tags, take the first
non-empty line, strip a leading `Title:`/`Name:`/`Conversation:` prefix and
surrounding quotes/trailing punctuation, cap at `MAX_WORDS = 5` words and
`MAX_CHARS = 48` characters. A reply that doesn't look like a title (more
than 15 words) or comes back empty yields `null`, and the caller leaves the
provisional title (first 48 chars of the message) alone rather than
overwriting it with garbage.

### 5.6 Listing / loading / resuming / forking / deleting — where each lives

- **List**: renderer holds `threads` in React state, hydrated wholesale on
  open (§3.4); no separate "list" round-trip at runtime beyond that initial
  hydration and the folder-watcher-triggered re-reads.
- **Load**: opening a thread is a pure in-memory operation (`messages[threadId]`)
  once hydrated — there is no per-open disk read.
- **Resume a turn across a window reload**: `agent:active` (`session/ipc.cjs:613`)
  returns every turn still `status: "running"` **from the in-process
  `records` map** (never from disk — a resume only makes sense for a turn
  this same process is still executing) with its `events` array; the
  renderer folds those events into the message it left behind through the
  *same* fold function the live stream uses, guaranteeing a rejoined turn is
  byte-identical to one that was never interrupted.
- **Fork**: not a first-class operation in this layer — a "fork" in the UI is
  implemented as `createThread` + copying the relevant prefix of `messages`
  into the new thread's initial state (renderer-side; no dedicated backend
  concept of a fork/branch exists in the storage layer).
- **Delete**: removing a thread removes both `conversations/threads/<id>.json`
  and `conversations/messages/<id>.json` (both drop out of the `SOURCES`
  read functions simultaneously when the thread leaves React state) and
  triggers `agent:forget(threadId)` to clear every other subsystem's
  per-thread state (permission grants, session-store finished turns, memory
  capture's "already covered" watermark, todo/task bookkeeping).

---

## 6. Compactions (context-window summarization cache)

`src/main/session/compactions.cjs` — **not** a durable conversation record;
purely a performance cache under `cache/compactions/`. Purpose: the renderer
resends the *entire* transcript on every turn; when the loop has to summarize
the oldest messages to fit the model's context window, that summary would
otherwise be recomputed (and re-billed) on every subsequent turn against a
transcript that starts the same way.

**Cache key**: SHA-1 of the thread id, first 24 hex chars, filename
`<hash>.json`.

**Cache entry**:
```jsonc
{
  "summary": "...",              // the compaction note text
  "covered": 42,                 // how many of the INCOMING transcript entries it stands for
  "fingerprint": "sha1-hex",     // SHA-1 over JSON.stringify of each of the first `covered` entries
  "at": 1735689600000            // epoch ms
}
```
**Validity check** (`recall`, `compactions.cjs:105-115`): a stored entry is
usable only if `history.length >= stored.covered` **and**
`fingerprint(history.slice(0, stored.covered)) === stored.fingerprint`. Any
mismatch (a message was edited or deleted, or history got shorter) drops the
cache entirely (`forget`) rather than trying to patch it — "wrong-and-dropped
is the only failure mode, and it costs one summary, which is what it cost
before" (`compactions.cjs:20-23`).

**Retention**: `sweep({ days: 60 })` deletes cache files untouched (by
mtime) for 60+ days; not run automatically inside this module — must be
invoked by a maintenance/startup path (worth confirming call site if porting
this exactly; not critical to correctness either way since it's pure cache).

**Rust port**: this is safely optional for a first pass — dropping it only
costs re-paying for a compaction summary on every turn of a very long
conversation, never a correctness issue. If ported, keep the same SHA-1
fingerprinting scheme so a workspace shared between old and new builds
doesn't thrash the cache.

---

## 7. Snapshots

`src/main/snapshot/index.cjs` — opencode's approach, borrowed deliberately
(per the codebase's own convention of tracking opencode as a reference
implementation): a **shadow git repository** per project folder, whose work
tree *is* the project and whose object store lives entirely under
`cache/snapshots/<key>/` in the workspace — never inside the project, never
visible to the project's own `.git`.

- **Key**: `sha1(resolve(cwd).toLowerCase()).slice(0,16)` — one shadow store
  per distinct working directory.
- **Environment**: `GIT_DIR=<store>`, `GIT_WORK_TREE=<resolved cwd>`,
  `GIT_CONFIG_GLOBAL`/`GIT_CONFIG_SYSTEM` pointed at `NUL`/`/dev/null` so no
  user git config (hooks, signing, global excludes) leaks in.
- **What a snapshot captures**: `git add -A -- .` then `git write-tree` — a
  **tree object**, deliberately not a commit (no parent, no branch, nothing
  the project's real git ever sees). The project's own `.gitignore` applies
  (git reads it from the work tree regardless of `GIT_DIR`); an additional
  hardcoded exclude list is written to `<store>/info/exclude`:
  `node_modules/`, `.git/`, `.next/`, `dist/`, `build/`, `out/`, `target/`,
  `__pycache__/`, `.venv/`, `venv/`, `.cache/`, `.turbo/`, `*.log` — plus, if
  the workspace folder happens to be *inside* the project being snapshotted,
  the workspace's own relative path is excluded too (so snapshotting doesn't
  recursively capture its own snapshot store).
- **Size guard**: `MAX_FILES = 50_000` — if `git ls-files` after the add
  exceeds this, the snapshot is abandoned (`track` returns `null`) rather
  than let one turn stall on a huge tree.
- **When taken**: `SNAPSHOT_BEFORE = {"write","edit","patch","shell"}`
  (`session/index.cjs:126`) — before the *first* such tool call in a turn,
  lazily (a read-only turn never pays for `git add -A`).
- **`changes({root, cwd, from, to?})`**: diffs two trees (`to` defaults to a
  fresh `track()` of the current work tree — "what did this turn change,
  right now"). Returns `{files: [{path, status, from(if renamed), additions,
  deletions}], to}` via `git diff --name-status -M` + `git diff --numstat -M`.
- **`diff({root, cwd, from, to, file?})`**: the actual unified patch text for
  one file or the whole change (`git diff -M --no-color`).
- **Restore semantics** (`revert`, `snapshot/index.cjs:227-261`): computes
  which files exist now but didn't in tree `to` (`git diff --name-only
  --diff-filter=A to now`) — those are files the turn *created* — then
  `git read-tree <to>` + `git checkout-index -a -f` to restore every file's
  content, then explicitly `rm`s each created file (a plain tree checkout
  does not delete files absent from the target tree; the created-files
  diff is what makes revert actually reverse a turn instead of only
  overwriting what existed before). Finally re-snapshots so the next
  comparison starts from the reverted state. Files git never saw (ignored
  ones) are untouched either way.
- **Not a backup**: the object store is explicitly disposable — "safe to
  delete" — and the app makes no promise about it surviving a `cache/` wipe.

**Rust port**: shell out to a system `git` exactly as today (or use
`git2`/`gix` for an in-process implementation — either is viable; shelling
out is simplest to keep behaviorally identical, since the exact flag set
matters for edge cases like rename detection `-M`). Preserve the
`GIT_DIR`/`GIT_WORK_TREE`/exclude-file scheme and the `MAX_FILES` guard
verbatim.

---

## 8. Memory: storage, retrieval, and prompt injection

### 8.1 Storage

One JSON file per memory in `memory/<id>.json`, schema in §4.16. Two
"providers" behind a common interface (`src/main/memory/index.cjs`,
`providers/local.cjs`, `providers/mcp.cjs`):

- **local** (default): the workspace's own `memory` collection.
- **mcp**: delegates list/recall/remember/update/forget to an external MCP
  memory server (configured via `preferences.memoryBackend`), falling back to
  `local` if the remote backend is unreachable or misconfigured — memory
  degrades, it never blocks a turn.

### 8.2 Retrieval / scoring — `src/shared/memory.js`

Pure, dependency-free (shared verbatim between main and renderer — the
renderer needs the same ranking to render the Memory screen sensibly). BM25
over `title` (weighted ×2) + `tags` (×2) + `description` + `body`, with a
short English stop-word list. `score()` combines BM25 relevance + a pin bonus
(`pinWeight = 1.5`) + a gentle recency-decay bonus (`freshWeight = 0.6`,
90-day half-life-ish curve via `1 / (1 + days/90)`), tie-broken by id.
`recall(query, {limit, touchUsed})` (as used by an agent's explicit
`memory_recall` tool) filters to `relevance > 0` only — no query match means
no results, deliberately (returning "closest" results on an empty match would
look like working search while providing none). Recalling bumps `useCount`
and `lastUsedAt` (fire-and-forget, failure doesn't fail the recall).

**Deduplication** (`findDuplicate`/`merged`, `memory.js:400-452`): scoped
first (same `scope`, and for `project` scope the same `folder` via
`sameFolder` — case/slash-insensitive path comparison), then same normalized
title **or** ≥0.8 Jaccard-style term-overlap similarity between
`title+body`. A duplicate write **merges**: new wording wins, but `tags` are
unioned (capped at 8), `pinned` is OR'd, `createdAt`/`useCount` are preserved
from the older record — "a repeat with different wording usually is a
correction," never a reason to lose accumulated metadata.

**Secret filtering** (`looksSecret`/`isStorable`, `memory.js:472-495`):
regexes for OpenAI/GitHub/Slack/AWS/Google API key shapes, PEM private-key
headers, JWT-shaped strings, and a generic `key|secret|token|password\s*[:=]\s*<opaque>`
pattern. Checked **before** writing, never after — a memory record failing
this check is simply never stored.

### 8.3 Injection into the prompt — `forTurn` (`memory/index.cjs:113-160`)

Computed **once**, at the top of a turn, and never rebuilt mid-turn — the
comment is explicit about why: the system prompt (which includes the memory
block) sits at the front of the provider's cacheable prefix, and changing it
mid-conversation breaks prompt-cache reuse for everything after it, costing
far more than the freshness gained.

Pipeline: `list({limit:500})` → filter to `applicable(records, folder)`
(project-scoped memories only apply in their own folder, matched via
`sameFolder`; a project memory with no `folder` recorded is treated as
global rather than unreachable) → filter to `approved` (drop `pending: true`
— a memory awaiting review must not be *used* just because it's visible) →
optionally filter to the current `agentId` if `memoryPerAgent` is on → rank
via `forPrompt(scoped, {query: latestUserText(history), budget})`.

`forPrompt` (`memory.js:307-363`) ranks in **three tiers**, strictly ordered:
(0) pinned memories and the project's `handover` note, always first; (1)
memories that actually score against the just-sent query; (2) everything
else, by `useCount` then recency — a small baseline so a message sharing no
words with a relevant memory still surfaces something proven useful before.
Cut to a **byte budget** (`INJECT_BUDGET_BYTES = 2560` default, one line
`"<title>: <summary>"` per memory, `summarise()` capped at 160 chars) rather
than a fixed count — a pinned memory is never dropped for length, only what
else fits around it is affected.

The resolved folder (needed only if any memory is project-scoped, to avoid
paying for a `.git`-walk on every turn of an all-global workspace) comes from
`prompt.projectRoot(cwd)`, called lazily via a passed-in `resolveFolder`
function.

### 8.4 Capture — noticing what's worth remembering, after the fact

`src/main/memory/capture.cjs` — the hard problem this solves is that a
conversation doesn't have a clean "end" event; it goes quiet and might resume
any time. Mechanism: a per-thread timer, **armed** when a turn finishes
(`session/ipc.cjs:454-477`, right after the loop resolves) and **disarmed**
the instant the next turn on that thread starts (`ipc.cjs:299`). Default wait
`IDLE_MS = 4 minutes`; shortened to `TURN_MS = 20 seconds` if the user's
`memoryCapture` preference is `"turn"` (capture after every message). Any
timer still armed when the app quits is drained with a `DRAIN_MS = 12s`
budget in the teardown chain (`drain()`).

When the timer fires (`run(threadId)`): guarded three ways — feature enabled,
capture mode not `"off"`, and the conversation must not currently be busy
(re-arms and waits if so, rather than capturing mid-sentence). Only the
transcript **since the last successful capture** is sent (`coveredFor`
watermark, counted in message-count terms) — a conversation that goes quiet
twice is not re-read from the top. Calls `extract.buildPrompt(...)` (system +
user prompt asking the model what's worth keeping, given the existing
memory titles for this scope so it can supersede rather than duplicate) via
the same provider/model the conversation itself used (no separate
extraction model configured), parses the reply, filters through
`isStorable` (§8.2 secret check), and writes each via `remember`
(merge-aware) or `update` (if it names an existing memory id it supersedes).
A capture pass never throws into the app — every failure path returns
`null` silently; the user simply doesn't get memories written that turn.

**Rust port**: this whole subsystem is architecturally a background job
scheduler keyed by thread id with idle-timeout semantics; a `tokio::time`-based
per-thread debounce timer plus a `HashMap<ThreadId, JoinHandle>` (cancel =
abort the handle) reproduces it directly. The "drain on quit with a budget"
behavior maps to a `tokio::time::timeout` around `futures::future::join_all`
of the armed threads' capture futures.

---

## 9. Atomicity, concurrency, corruption recovery, migration

### 9.1 Atomicity — summarized from §2

Every write is temp-file + best-effort fsync + atomic rename, confined
inside the workspace root, with retry-on-contention for Windows sharing
violations. This is the one piece of the port that must not be weakened:
several documented production incidents (silent NUL-byte files, a "deleted"
conversation reappearing) trace directly to skipping steps here.

### 9.2 Concurrency model

There is **no cross-process locking** anywhere in this codebase — the
concurrency story is entirely "one Electron main process owns the workspace,
one renderer process owns live conversation state, and the two talk over
IPC." Two things stand in for real concurrency control:

1. **Debounced, diffed writes** (§3.4) minimize write volume but do not
   prevent a race between, say, an agent's `write` tool editing
   `agents/x.json` directly via `fsx` and the renderer's own debounced sync
   of the `agents` collection landing moments later. The **folder watcher**
   (§3.3) is what reconciles this — not locking, but "whoever wrote last on
   disk wins, and everyone else re-reads." The renderer's `written` map
   tracks its own last-known fingerprint per record specifically so it can
   tell "this changed because I wrote it" from "this changed because
   something else did" and avoid re-writing what it just read.
2. **Retry-on-contention** (§2.3) handles transient external lock-holders
   (antivirus, indexers) but not genuine concurrent writers.

**Two Inertia installs pointed at the same workspace folder simultaneously
is not a supported configuration** — nothing in the code defends against it
beyond "last writer wins" at the file level. The Rust port should preserve
this scope (document it as a known limitation rather than silently trying to
add real locking, which would be new behavior, not a faithful port) unless
the product decision changes.

### 9.3 Corruption recovery

Covered in §2.4: unparsable JSON is preserved as `<file>.damaged` (once) and
the caller proceeds with a safe fallback (usually an empty collection/array,
never a crash). There is no automatic *repair* — `src/main/workspace/repair.cjs`
exists and is run once at startup (`workspace/index.cjs:117-131`,
`repair.repair(configured)`) but its job (per the file, not re-derived here
in full) is structural cleanup (e.g., dangling references), not JSON
un-corruption — a `.damaged` file is left for a human, never auto-fixed.

### 9.4 Migration / versioning

`src/main/workspace/migrate.cjs`. `WORKSPACE_VERSION` (currently `1`,
`layout.cjs:15`) is stamped in `inertia.json`'s `version` field. Rules:

- A folder claiming a **higher** version than this build understands is
  **refused outright** — `migrate()` returns `{ok:false, reason:"newer", ...}`
  with a message telling the user to update the app or choose a different
  folder. This is a hard stop specifically to prevent an old build from
  "helpfully" relabeling a newer folder as old and corrupting it with
  obsolete rules (`migrate.cjs:14-21` documents this as a real bug that was
  fixed).
- A folder at the **current** version: no-op (`{ok:true, migrated:false}`).
- A folder **behind** the current version: only proceeds if `MIGRATIONS`
  (currently an **empty array** — the format hasn't changed since v1) has a
  contiguous run of steps covering `from+1 .. WORKSPACE_VERSION`; otherwise
  refused as `{reason:"no-path"}`. If covered: **back up the entire folder
  first** (plain recursive copy into `backups/v<from>-<timestamp>/`,
  excluding `backups/`, `logs/`, `.git/`, `.tmp/`, `node_modules/` — a real
  copy of real files, deliberately not an archive, so a person can open it
  without the app), then run each step's `run(root)` in order, then stamp the
  new version.
- Each migration step must be **idempotent** on a partially-applied folder —
  the comment is explicit that an interrupted migration is retried from the
  backup, so doing half of it twice must not be worse than doing it once.

**Rust port recommendation**: introduce a `SCHEMA_VERSION` constant from day
one and implement this exact three-way check (refuse-newer /
no-op-current / backup-then-migrate-older) even though there is currently
nothing to migrate — this is the seam the Rust rewrite itself will need the
moment its own on-disk shape needs to change, and matching the existing
manifest field name (`version` inside `inertia.json`) means an *existing*
Electron-created workspace opens correctly in the Rust build without a
special-case "is this an old-format folder" branch.

---

## 10. Rust design notes

### 10.1 Core serde structs

```rust
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Envelope fields every `json`-kind collection record gets from `collections.put`.
/// Prefer composing this in rather than repeating the three fields everywhere,
/// e.g. via `#[serde(flatten)]` on a wrapper, or a trait with default methods.
pub trait Record {
    fn id(&self) -> &str;
    fn created_at(&self) -> &DateTime<Utc>;
    fn updated_at(&self) -> &DateTime<Utc>;
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Thread {
    pub id: String,
    pub agent_id: Option<String>,
    pub title: String,
    pub mode: ConversationMode,          // chat | plan | autonomous
    pub approval: Approval,              // ask | edits | auto
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub unread: u32,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub message_count: u32,
    pub computer_attached: Option<String>,
    pub created_at: DateTime<Utc>,
    // draft/temporary are renderer-side-only states and MUST NOT be
    // (de)serialized — a record reaching disk is by definition neither.
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Agent,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message {
    pub id: String,
    pub role: String,                    // "user" | "agent" — see note below
    pub agent_id: Option<String>,
    #[serde(default)]
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub status: MessageStatus,           // sent | streaming | error
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MessagePart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub stopped: bool,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<serde_json::Value>, // current-turn-only; do not persist across turns
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessagesFile {
    pub id: String,       // == thread_id
    pub thread_id: String,
    pub messages: Vec<Message>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Turn {                        // history/sessions/<id>.json
    pub id: String,
    pub thread_id: Option<String>,
    pub message_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub model_ref: Option<String>,
    pub cwd: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub status: TurnStatus,              // running | complete | error | cancelled | interrupted
    #[serde(default)]
    pub steps: u32,
    pub usage: Option<Usage>,
    pub events: Vec<serde_json::Value>,  // heterogeneous event union — see note
    #[serde(default)]
    pub truncated: bool,
    // `root` is NEVER serialized — stripped before write in the JS original.
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MemoryRecord {
    pub id: String,
    pub title: String,
    pub body: String,
    pub description: Option<String>,
    pub kind: MemoryKind,
    pub scope: MemoryScope,              // global | project
    pub folder: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: MemorySource,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub pending: bool,
    pub agent_id: Option<String>,
    #[serde(default)]
    pub use_count: u32,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

Notes:
- `Turn.events` is genuinely a heterogeneous union (`delta`, `tool-start`,
  `tool-end`, `warning`, `hook`, `error`, `done`, `changes`, `steer`, each
  with different fields) — either model it as a `#[serde(tag = "type")]` enum
  with one variant per event kind (more type-safe, recommended if the Rust
  side also *renders* these events, e.g. in a native history viewer) or keep
  it as `serde_json::Value` (simpler, matches "we just need to write this
  back out unchanged" for a first port pass that doesn't reimplement the
  agent loop's own event semantics yet).
- Field naming: this document uses `snake_case` per Rust convention: apply
  `#[serde(rename_all = "camelCase")]` at the struct level (or per-field
  `#[serde(rename = "...")]` where the JSON key doesn't mechanically map,
  e.g. `agent_id` → `agentId`) so the wire/file format stays camelCase JSON,
  matching every existing file on disk exactly.
- `DateTime<Utc>` for ISO-8601 timestamp fields — confirm `chrono`'s default
  serde output matches `new Date().toISOString()` exactly (millisecond
  precision, `Z` suffix, no offset notation) or use `time` crate with an
  explicit ISO-8601 format string; a mismatched timestamp format would make
  an existing workspace's `updatedAt`-sorted lists silently misorder.

### 10.2 Repository / store trait seam

Two layers, matching the existing JS architecture's own split:

```rust
/// Layer 1 — mirrors fsx.cjs. The only thing that touches a filesystem.
#[async_trait]
trait FileStore: Send + Sync {
    async fn read_json<T: DeserializeOwned>(&self, path: &Path) -> Result<Option<T>>;
    async fn write_json<T: Serialize + Sync>(&self, path: &Path, value: &T) -> Result<()>;
    async fn read_text(&self, path: &Path) -> Result<Option<String>>;
    async fn write_text(&self, path: &Path, contents: &str) -> Result<()>;
    async fn list_dir(&self, dir: &Path, only_dirs: bool, ext: Option<&str>) -> Result<Vec<String>>;
    async fn remove(&self, path: &Path) -> Result<()>;
    fn resolve_inside(&self, root: &Path, rel: &str) -> Result<PathBuf>;
}

/// Layer 2 — mirrors collections.cjs. One implementation wraps a FileStore
/// (today's behavior); a hypothetical SQLite-backed implementation (§10.3)
/// would implement the same trait with zero changes required upstream.
#[async_trait]
trait CollectionStore<T: Serialize + DeserializeOwned + Record + Send + Sync>: Send + Sync {
    async fn ids(&self) -> Result<Vec<String>>;
    async fn get(&self, id: &str) -> Result<Option<T>>;
    async fn list(&self) -> Result<Vec<T>>;   // per-record errors swallowed, matching collections.cjs:101-106
    async fn put(&self, record: T) -> Result<T>;
    async fn patch(&self, id: &str, changes: serde_json::Value) -> Result<T>;
    async fn remove(&self, id: &str) -> Result<()>;
    async fn rename(&self, id: &str, next_id: &str) -> Result<T>;
}

/// Documents (single-file, no id/timestamps) — mirrors readDocument/writeDocument.
#[async_trait]
trait DocumentStore: Send + Sync {
    async fn read<T: DeserializeOwned + Default>(&self, key: &str) -> Result<T>;
    async fn write<T: Serialize + Sync>(&self, key: &str, value: &T) -> Result<()>;
}
```

This seam is what makes the storage layer mockable for tests (an
in-memory `FileStore` backed by a `HashMap<PathBuf, Vec<u8>>`, matching the
spirit of the existing `session.test.js` suite which stands up real temp
directories) and swappable per-collection without touching call sites
(session loop, Tauri command handlers, the memory subsystem).

### 10.3 Should any of this move to SQLite?

**Recommendation: no, not for the primary record store — keep the
JSON-files-per-record model as the ground truth, exactly as today.**

Reasoning:

1. **The folder-as-product constraint is explicit and load-bearing**, not
   incidental. `docs/workspace.md` and the in-code comments repeatedly frame
   "a folder you can read, diff and copy" as *the* product requirement — git
   syncing a workspace, hand-editing a JSON file, and backup-by-copy are all
   documented, intended workflows, not accidents of the implementation.
   SQLite in a single `.db` file breaks every one of those: it isn't
   diffable in git in any useful way, isn't hand-editable, and a raw file
   copy of a live SQLite database while a writer holds the file is not
   guaranteed consistent the way copying a tree of finished, atomically-
   renamed JSON files is.
2. **Scale doesn't currently demand it.** This is a single-user, local-first
   desktop app. "Hundreds of threads, thousands of memories, tens of
   thousands of turn records" is comfortably inside what `readdir` + parse
   handles in milliseconds; there is no evidence in the codebase of this
   being a measured bottleneck (no caching layer exists specifically to
   paper over collection-list cost, unlike, say, the memory BM25 index which
   *is* explicitly built to avoid recomputation).
3. **The task's own constraint reinforces this**: "the folder layout must
   stay the same." Any SQLite adoption for records users currently browse as
   files (agents, skills, memory, conversations) would visibly break that
   contract for anyone who has ever opened their workspace folder.

**Where SQLite (or another embedded engine) legitimately earns its place**,
scoped narrowly and always as a *derived index*, never the source of truth:

- **Full-text/semantic search across memory and conversation history**, if
  the product ever wants better-than-BM25-over-500-records search at larger
  scale. Store it as `cache/index.sqlite`, rebuildable from the JSON files at
  any time (treat it exactly like `cache/snapshots/` — safe to delete,
  rebuilds on demand), never authoritative.
- **The turn/session event log** (`history/sessions/*.json`), if this
  subsystem grows in ways JSON-per-turn stops serving well (e.g., a "search
  all my past turns for where I used tool X" feature) — again as a rebuilt
  index, or as an **additive** SQLite table that's written alongside the
  existing JSON file rather than replacing it, so the JSON stays the
  portable/durable copy and SQLite is purely a query accelerator.
- If the product direction changes such that true multi-writer concurrency
  (§9.2) becomes a real requirement (e.g., a sync server, multiple devices
  editing live), that is the point to seriously reconsider SQLite (with
  WAL mode) or another embedded engine *for the collections that need
  transactional guarantees* — but that is a product decision with UX
  consequences (loses "just a folder of files"), not a mechanical storage
  port decision, and is out of scope for a faithful port.

**If asked to prototype SQLite for something**, implement it strictly behind
the `CollectionStore<T>` trait (§10.2) as an alternate backend selectable per
collection, so the on-disk `.json` files remain the default and the folder
contract is preserved unless a user/product explicitly opts a given
collection into a different backend.

### 10.4 File-locking / async-IO concerns

- **No file locks exist today** (§9.2) — the Rust port should not invent
  them as a "safety improvement" without a product decision, since that
  changes observable behavior (e.g., two processes racing to write the same
  file today silently have "last write wins"; introducing locks would turn
  that into either blocking or an error, both new behavior).
- **All file I/O should be async** (`tokio::fs`), matching the Node
  `fs/promises` usage throughout — this is a UI-responsiveness requirement
  (Electron's main process is single-threaded for IPC handling; Tauri's
  async runtime has the same shape) more than a correctness one.
- **fsync-before-rename must be preserved** (§2.2) — this is the one place
  where "just write the file" is a known, previously-shipped data-loss bug.
  Use `tokio::fs::File::sync_all()` before closing the temp file handle, and
  treat its error as non-fatal exactly as the JS does (some filesystems —
  network shares, some virtual drives — legitimately reject fsync).
- **The Windows contention retry (§2.3) is not optional** — omitting it
  reproduces a previously-shipped, previously-fixed bug (files reappearing
  after "successful" deletion). Implement it with the same error-code set
  and the same delay ladder for behavioral parity; there's no evidence a
  different ladder would be worse, but there's also no reason to diverge
  without cause.
- **Debounce timers** (400ms for renderer→disk sync, 500ms for turn-record
  writes) should be preserved as tunable constants rather than hardcoded
  magic numbers, matching how they're already named constants in the source
  (`WRITE_DELAY` in two different files with two different values — do not
  conflate them, they debounce different things).
- **The folder watcher (§3.3)** needs an async, debounced, recursive
  filesystem watch — the `notify` crate's `RecommendedWatcher` in recursive
  mode is the direct equivalent of Node's `fs.watch(..., {recursive:true})`;
  add the same self-write suppression logic (compare against a
  last-known-fingerprint map) to avoid an infinite write→notify→reread loop,
  since `notify` does not inherently distinguish "I wrote this" from
  "something else wrote this" any more than `fs.watch` does.
