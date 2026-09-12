# 04 — External Tool Integrations (MCP, OpenAPI, Composio)

Source of truth (Electron/Node, read-only reference for this spec):

- `D:\oss\inertia\src\main\mcp\{index.cjs,protocol.cjs,transport.cjs,defaults.cjs,mcp.test.js}`
- `D:\oss\inertia\src\main\openapi\{index.cjs,spec.cjs,openapi.test.js}`
- `D:\oss\inertia\src\main\composio\{index.cjs,api.cjs,logos.cjs,composio.test.js}`
- Supporting: `src\main\tools\tool.cjs` (the tool contract), `src\main\tools\registry.cjs` (how sources compose), `src\main\workspace\{collections.cjs,secrets.cjs,layout.cjs}` (on-disk storage), `src\main\window-send.cjs` (IPC event fan-out).

All three integrations exist for one reason: turn something external (a local
MCP server, a REST API described by an OpenAPI document, a Composio-hosted
SaaS connection) into objects that satisfy the same **tool contract** as the
built-in `read`/`shell`/`grep` tools, so the agent loop never has to know
where a tool came from.

---

## 0. The shared tool contract (all three integrations produce this)

`src\main\tools\tool.cjs:71-99` (`defineTool`) — every tool, from any source, is:

```js
{
  id: string,                 // unique across every source, what the model calls
  description: string,        // read by the model to decide whether to call it
  parameters: JSONSchema,     // sent to the provider, used to validate the call
  permission: {
    key: string,               // e.g. "mcp", "openapi", "composio"
    target: (args) => string,  // the specific thing a rule is written about
    always: (args) => string,  // optional, what "always allow" remembers
  },
  source: "builtin" | "mcp" | "openapi" | "composio",
  render: (args) => string | null,   // UI label while running
  execute: async (args, ctx) => { title, output, metadata, images? },
  normalize: (rawArgs, ctx) => rawArgs | null,  // optional pre-schema fixup
}
```

`runTool` (`tool.cjs:109-192`) is the single call path for every tool
regardless of source:

1. `normalize(rawArgs, ctx)` if present (never throws out — falls back to
   `rawArgs` on error).
2. JSON-Schema validate against `parameters` (`schema.cjs`); a failure
   returns `{ ok:false, output: "Invalid arguments for <id>: <error>" }`
   **without calling execute** — this never reaches the tool.
3. `ctx.ask({ key, target, always, title, args })` — the permission gate.
4. `tool.execute(args, ctx)`.
5. `truncate.output(result.output, { root, toolId })` — output-size capping
   shared by every tool.
6. Returns `{ ok, title, output, images?, metadata, durationMs }`. A thrown
   `ToolError`/generic `Error` inside `execute` is caught and turned into
   `ok:false` with `output: error.message` — **it never kills the turn**,
   except cancellation (`ctx.signal.aborted`), which rethrows.

`src\main\tools\registry.cjs:85-112` — how the three sources are wired into
one flat list:

```js
const SOURCES = [
  { name: "mcp",     module: "../mcp/index.cjs",     fn: "mcpTools" },
  { name: "openapi", module: "../openapi/index.cjs", fn: "openapiTools" },
  { name: "composio", module: "../composio/index.cjs", fn: "composioTools" },
];
```

`fromSources(root)` (`registry.cjs:93-112`) calls each `fn(root)` in a
try/catch — **a broken source is dropped with its error recorded in
`problems`, never thrown** — then concatenates with `builtins` and **sorts
the combined list by `tool.id`** (registry.cjs:105-111). This sort exists
purely for prompt-cache locality: two turns whose tool list differs only in
order share no cached prefix, so a stable deterministic order matters more
than it looks like it should. `forAgent()` further filters by permission
rules (`registry.cjs:145-188`) before tools reach a model.

**Rust design note:** this is the seam. Define a `ToolProvider` trait once;
`Registry::all()` collects `Vec<Box<dyn Tool>>` from builtins + each
provider, sorts by id, and `forAgent()` filters by permission. Each of MCP /
OpenAPI / Composio becomes one implementation of the provider trait that
returns `Vec<Tool>` (or a stream) built from its own live state — see §5.

```rust
#[async_trait]
trait ToolProvider: Send + Sync {
    /// Human name used in problems map ("mcp", "openapi", "composio").
    fn name(&self) -> &'static str;
    /// Called on (roughly) every turn; must be cheap — read cached state,
    /// never touch the network/process directly.
    async fn tools(&self, workspace: &Path) -> anyhow::Result<Vec<Tool>>;
    /// Drop any cached tool list (called when a connection/import changes).
    fn invalidate(&self, scope: Option<&str>);
}

struct Tool {
    id: String,
    description: String,
    parameters: serde_json::Value,   // JSON Schema
    permission: Permission,
    source: &'static str,
    render: Option<Box<dyn Fn(&serde_json::Value) -> String + Send + Sync>>,
    execute: Box<dyn Fn(serde_json::Value, ToolCtx) -> BoxFuture<'static, Result<ToolResult, ToolError>> + Send + Sync>,
    normalize: Option<Box<dyn Fn(serde_json::Value, &ToolCtx) -> serde_json::Value + Send + Sync>>,
    metadata: serde_json::Value,
}
```

---

## 1. MCP (Model Context Protocol)

Files: `mcp/index.cjs` (882 lines — connections + IPC + tool adaptation),
`mcp/protocol.cjs` (508 lines — pure JSON-RPC, framing, schema sanitizing,
content flattening), `mcp/transport.cjs` (326 lines — stdio + Streamable
HTTP transports), `mcp/defaults.cjs` (99 lines — folder-argument
auto-fill).

### 1.1 Transports supported

Two, both behind the same 5-method interface (`transport.cjs:9-16`):

```js
start()        // bring the connection up, or throw saying why not
send(message)  // write one JSON-RPC message
onMessage(fn)  // called with each message that arrives
onClose(fn)    // called once, with an Error if it died rather than ended
close()        // tear it down
```

**Stdio** (`transport.cjs:69-204`, `stdioTransport`):
- Spawns a child process via `child_process.spawn`.
- On Windows, cannot spawn `.cmd`-based commands (`npx`, `uvx`, etc.)
  without going through a shell, and Node's own `shell:true` doesn't quote
  correctly, so `windowsCommandLine()` (line 61-67) hand-builds and quotes
  the command line, then spawns `cmd.exe /d /s /c "<quoted line>"` with
  `windowsVerbatimArguments: true`.
- On POSIX, spawned directly with `detached: true` (own process group) so
  `close()` can `process.kill(-pid, "SIGTERM")` to kill the whole tree
  (shim + real server); Windows uses `taskkill /pid <pid> /T /F`.
- SIGTERM then a 2s-later `SIGKILL` fallback if the process ignores it
  (line 189).
- Framing: newline-delimited JSON (`protocol.cjs:234-252`, `parseFrames`) —
  buffers a partial trailing line across chunks, silently drops lines that
  fail to `JSON.parse` (servers sometimes print non-protocol noise to
  stdout).
- Stderr is captured in a ring buffer of the **last 8000 chars**
  (`STDERR_LIMIT`, transport.cjs:34) — this is the primary diagnostic for a
  server that dies before speaking JSON-RPC (a bad token, missing runtime,
  etc. produce a stack trace on stderr and a non-zero exit, not a
  JSON-RPC error).
- `CONNECT_TIMEOUT_MS = 30000` (line 23) — spawn must produce a pid or an
  error within 30s.
- Env: `{ ...process.env, ...record.env }` — inherits full parent env
  (PATH, HOME, proxy settings) with the record's own env layered on top.

**Streamable HTTP** (`transport.cjs:217-318`, `httpTransport`):
- Every message (including `initialize`) is a POST to the server's URL.
  No persistent connection — `start()` only validates the URL parses.
- Request headers: `Content-Type: application/json`,
  `Accept: application/json, text/event-stream`, plus any static headers
  from the record, plus `Mcp-Session-Id` once the server has issued one
  (read from response header `mcp-session-id`, stored and replayed on every
  later POST).
- Response is either a single JSON body, or `text/event-stream` — SSE
  events carrying JSON-RPC, parsed with `sseFrames()` (`protocol.cjs:261-273`,
  reuses the same `sseEvents` splitter as the LLM streaming client).
  204/empty body means "this was a notification, no reply expected."
- **Redirects are refused** (`redirect: "error"`) — a redirect would carry
  the `Authorization` header to a host the user never named.
- `CONNECT_TIMEOUT_MS = 30000` is reused as the per-request abort timeout.

There is **no SDK dependency** — this is deliberate per the file header
comment: the wire format is "a hundred lines of framing and a Map."

### 1.2 JSON-RPC handshake and capability negotiation

`createPeer()` (`protocol.cjs:38-220`) implements JSON-RPC 2.0 client-side
correlation:
- `request(method, params, {signal, timeoutMs, onProgress})` — assigns an
  incrementing integer id (not UUID — "what every server in the wild echoes
  back... and reads well in a log"), stores `{resolve, reject, timer,
  onProgress}` in a `pending: Map<id, entry>`, sends
  `{jsonrpc:"2.0", id, method, params: {...params, _meta: {progressToken: id}}}`
  (the `_meta.progressToken` is only attached when `onProgress` is supplied).
- `REQUEST_TIMEOUT_MS = 60000` default per-request timeout
  (`protocol.cjs:22`); a per-server override is read from the record's
  `timeoutMs` field when connecting (`index.cjs:538-540`).
- **Progress-as-heartbeat**: a `notifications/progress` message whose
  `progressToken` matches a pending request id calls `entry.rearm()`,
  pushing the timeout deadline out by another full interval
  (`protocol.cjs:130-146`). This lets a long-running tool call (an import,
  a build) survive past the nominal timeout as long as the server keeps
  emitting progress.
- Server-initiated requests (id + method, e.g. `sampling/createMessage`,
  `roots/list`, `elicitation/create`) are **not implemented**: the peer
  replies with JSON-RPC error `-32601` ("Inertia does not implement
  <method>.") rather than leaving the server hanging (`protocol.cjs:169-183`).
- Abort: a passed `AbortSignal` rejects the pending request with
  "The request was cancelled." and detaches its listener.

Handshake sequence (`index.cjs:187-304`, `connect()`):
1. `resolveConfig()` — resolve `{secret:NAME}` references in `env`/`headers`
   via `secrets.resolve()` (never written back to the record).
2. Build transport (`transportFor()`, line 126-136): `record.type === "http"`
   → `httpTransport`, else `stdioTransport`.
3. `transport.start()`.
4. `peer.request("initialize", { protocolVersion, capabilities: {}, clientInfo: { name: "Inertia", version } })`
   where `protocolVersion = "2025-06-18"` (`PROTOCOL_VERSION`, line 44) and
   `clientInfo.version` comes from `app.getVersion()`.
5. `peer.notify("notifications/initialized")` — **fire-and-forget**, sent
   immediately after `initialize` resolves; the server must not be queried
   further until this fires ("works against lenient servers and hangs
   against strict ones").
6. `entry.capabilities = initialized.capabilities`,
   `entry.serverInfo = initialized.serverInfo`.
7. If `capabilities.tools` is truthy: `listAllTools(peer)` (line 165-177) —
   pages `tools/list` with `{cursor}` until `nextCursor` is falsy or
   `MAX_TOOL_PAGES = 20` pages are exhausted (protects against a
   looping/misbehaving server).
8. Status transitions to `"connected"`; the workspace record is patched
   with `{status:"connected", error:"", toolCount}`.

Client capabilities sent in `initialize` are an **empty object** — this
client does not advertise sampling, roots, or elicitation support (and
correctly answers `-32601` if a server tries to use any of them).

### 1.3 Server lifecycle (spawn, initialize, health, restart, shutdown)

Live connection state (`index.cjs:63-80`): a `Map<recordId, entry>` where
`entry = { record, transport, peer, tools, capabilities, serverInfo,
status, error }`. This is **process state, not persisted** — the on-disk
record only remembers configuration + last-known status/error/toolCount.

**Statuses**: `"idle"` (never connected this run), `"connecting"`,
`"connected"`, `"failed"`, `"disabled"` (record has `enabled: false`).

**Supervisor** (`index.cjs:384-460`): a `setInterval` tick every
`SUPERVISE_INTERVAL_MS = 15000` (unref'd so it never blocks quit), plus one
immediate pass at `startSupervisor()`. Each tick:
- Reads the workspace root (skips silently if none is set yet).
- Lists all `plugins.mcp` records, filters to those that are
  `enabled !== false`, not in the `suppressed` set (user manually
  disconnected — sticky for the process lifetime, cleared on reconnect or
  edit), not already `connected`/`connecting`, and whose retry backoff has
  elapsed.
- Connects the due set **concurrently** via `Promise.all` (so N slow cold
  starts don't serialize).
- Re-entrancy guarded by a `supervising` boolean flag (one tick at a time).

**Backoff on failure** (`scheduleRetry`, line 99-103): exponential —
`RETRY_BASE_MS=5000 * 2^(attempts-1)`, capped at `RETRY_MAX_MS = 5*60*1000`
(5s, 10s, 20s, 40s... up to 5 min). Backoff state lives in a separate
`retries: Map<id, {attempts, dueAt}>`, explicitly **not persisted** — a
relaunch resets it (the user relaunching is read as "please try again now").

**Health**: no active polling/ping beyond the JSON-RPC request timeout
itself. Liveness is inferred from `transport.onClose` firing (child process
`exit` event, or the HTTP layer marking itself closed). A `notifications/message`
from the server is logged (`console.log`/`console.error` by level) but does
not affect status.

**Auto re-list on tool change** (`onNotification`, line 320-347): a
`notifications/tools/list_changed` from the server triggers a full
`tools/list` re-page; a failure during re-list **keeps the stale list**
rather than clearing it ("stale... is much better than empty").

**Restart**: `reconnect(workspace, id)` (line 377-382) = `disconnect()` then
`connect()` with the freshly-read record. `mcp:connect` IPC also clears
`suppressed`/`retries` for that id first.

**Shutdown** (`shutdown()`, line 370-375): stops the supervisor timer, then
`disconnect()`s every live entry — this is what kills orphaned child
processes on app quit (a stdio server is a spawned child that does **not**
exit when the parent does, unless explicitly killed).

**Unexpected close** (`transport.onClose` handler, line 221-246): rejects
every in-flight request via `peer.close(reason)`, marks the entry
`"failed"`, clears its tools, schedules a retry, logs to `failures.cjs`
(a shared cross-app failure log), persists the failure to the record, and
announces to the renderer.

### 1.4 Tool discovery and namespacing

`toolId(serverName, toolName)` (`protocol.cjs:290-292`):
```
`${sanitize(serverName)}_${sanitize(toolName)}`
```
where `sanitize()` (line 278) replaces everything outside
`[a-zA-Z0-9_-]` with `_` and lowercases. E.g. server "GitHub" + tool
`create_issue` → `github_create_issue`. This is **mandatory namespacing** —
without it two servers both exposing `search` would silently collide and
the model would call whichever loaded last with no error.

Tool-id collisions across the whole registry: `mcpTools()` (line 580-599)
tracks a `seen: Set<string>` and **first-registered wins** (deterministic
across reconnects, not "last wins").

**Schema sanitizing** for the model (`sanitizeSchema`, `protocol.cjs:409-426`,
built on `resolveSchemaRefs`, lines 332-370):
- Inlines every **local** `$ref` (`#/...` JSON Pointer) against the tool's
  own `inputSchema` document.
- A `$ref` already open on the current resolution path (self-reference,
  e.g. a comment-with-replies schema) collapses to
  `{ type:"object", description:"A value of the same shape." }` rather than
  recursing infinitely.
- A `$ref` pointing **outside** the document (a URL or external file) is
  **never fetched** (no network call at connect time to an unnamed host) —
  becomes `{ type:"object" }` and sets `unresolved = true`.
- Depth-capped at `MAX_SCHEMA_DEPTH = 12` — beyond that, `{}`.
- `stripSchema()` (line 372-389) removes `$schema`, `$id`, `$defs`,
  `definitions` (validator metadata some providers reject on a function
  schema), and `additionalProperties: true` (meaningless to a model, and
  strict-mode providers reject it) while **keeping** `additionalProperties:
  false` (makes a schema strict-mode-eligible).
- Root is coerced to `{ type:"object", properties:{} }` if the server sent
  something else — a function call's arguments are always an object.
- If `unresolved`, the tool description gets a trailing sentence: *"Part of
  its argument schema points outside the document the server sent, so that
  part is undescribed here. Pass what the server documents and it will
  validate the call."*

**Discovery is on the turn path but reads a cache**: `mcpTools(workspace)`
(line 580-599) returns a module-level `toolCache` (invalidated via
`invalidate()` whenever a connection changes) — no network I/O per turn.
Only `entry.status === "connected"` servers contribute tools.

### 1.5 Folder-argument auto-fill (`defaults.cjs`)

A connected server is one process for the whole app/workspace; the
directory it was spawned in has nothing to do with which project a given
turn is about. `fillFromContext(schema, args, ctx)` (`defaults.cjs:78-97`)
fills a **specific, hardcoded list** of property names —
`projectPath`, `project_path`, `projectRoot`, `project_root` — from
`ctx.cwd` (the session's working directory), but **only** when: the
property exists in the *sanitized* schema, is typed as (or includes)
`"string"`, and the call left it missing or blank. A value the model
explicitly supplied is always kept — this never overrides intent, only
fills gaps. Called from `normalize` on every MCP tool (`index.cjs:517`),
i.e. it runs before schema validation.

### 1.6 Tool call dispatch and result mapping

`toTool(entry, definition)` (`index.cjs:481-572`) wraps one MCP tool
definition:
- `execute(args, ctx)`: rejects with `ToolError` if the server entry is no
  longer `"connected"` at call time (a live re-check, not just at build
  time). Calls `peer.request("tools/call", { name, arguments: args ?? {} },
  { signal: ctx.signal, onProgress: () => {}, timeoutMs: record.timeoutMs
  || default })`. `onProgress` is a no-op listener supplied only so the
  peer sends a progress token — the tool contract has no channel for
  streaming progress back to the UI, so this exists purely to extend the
  deadline via the heartbeat mechanism (§1.2).
- Any JSON-RPC rejection is rethrown as `ToolError`.
- `flattenContent(result, limit)` (`protocol.cjs:442-495`,
  `MAX_RESULT_CHARS = 60000`) converts MCP's `content: [...]` array into a
  single text blob:
  - `type:"text"` parts → joined text lines.
  - `type:"image"`/`"audio"` → a placeholder line `[image: <mimeType>]` in
    the text, plus the raw base64 stashed in `attachments[]` (metadata,
    never sent to the model as text — "would be a megabyte of noise the
    model pays for and cannot read").
  - `type:"resource"`/`"resource_link"` → inlined if it has `.text`,
    otherwise a placeholder + attachment with `.blob`.
  - Unknown part types → `JSON.stringify(part)` (visible rather than
    silently dropped).
  - Falls back to `JSON.stringify(result.structuredContent)` if there's no
    `content` at all.
  - Output past `MAX_RESULT_CHARS` is hard-truncated with a
    `\n[result truncated]` marker (`truncated: true` flag propagated to
    metadata).
- `result.isError === true` (MCP protocol-level tool failure, distinct from
  a transport/JSON-RPC failure) → thrown as `ToolError` with the flattened
  text, or a generic "`<name>` failed and said nothing about why."
- Successful result: `{ title: "<server>: <tool>", output, metadata: {
  server, serverId, tool, attachments?, truncated? } }`.

**Permission key**: `{ key: "mcp", target: () => `${serverName}/${toolName}`,
always: () => `${serverName}/*` }` — this is what lets a permission rule be
written per-tool (`warehouse/query`), per-server (`warehouse/*`), or for
every MCP tool at once (`mcp` key with pattern `*`).

**App-internal calling (not via the model)**: `callTool(server, name, args,
opts)` (line 615-651) and `toolsOf(server)` (line 654-662) are a *separate*
un-flattened call path used by app code (e.g. a memory server rendering
rows needs the structured JSON, not the flattened transcript text). It
prefers `result.structuredContent`, falls back to `JSON.parse(text)`.

### 1.7 Resources / prompts

**Not implemented.** The client capability object sent in `initialize` is
empty (`{}`), so it does not declare `resources` or `prompts` support, and
nothing in `index.cjs`/`protocol.cjs` calls `resources/list`,
`resources/read`, or `prompts/*`. The only place "resource" content is
handled is as one of the possible **result** content-part types inside a
`tools/call` response (§1.6) — i.e. resources are only consumed when a tool
result happens to embed one, never proactively browsed.

**Sampling/roots/elicitation**: explicitly unimplemented server→client
requests, answered with JSON-RPC `-32601` (§1.2).

### 1.8 Config file format

Each server is one JSON file under `plugins/mcp/<id>.json`
(`layout.cjs:61`, via `collections.put/patch/get` — one file per record, no
database). Record shape (fields set in `addServer`, `index.cjs:674-703):

```jsonc
{
  "id": "github-mcp",                 // filename stem; assigned or user-given
  "name": "MCP server",               // display name; also the namespace prefix
  "type": "stdio" | "http",
  // stdio:
  "command": "npx",
  "args": ["-y", "@some/mcp-server"],
  "cwd": "",                          // optional working directory
  "env": { "TOKEN": "{secret:GITHUB_PAT}" },   // secret refs, never raw values
  // http:
  "url": "https://example.com/mcp",
  "headers": { "Authorization": "Bearer {secret:API_KEY}" },
  "enabled": true,
  "timeoutMs": 0,                     // 0 = use REQUEST_TIMEOUT_MS default (60000)
  "status": "idle",                   // "idle"|"connecting"|"connected"|"failed"|"disabled"
  "error": "",
  "toolCount": 0,
  "createdAt": "...", "updatedAt": "..."  // stamped by collections.put()
}
```

Secrets are stored **once**, centrally, in `secrets/secrets.json`
(`layout.cjs:94`) as `{ entries: { NAME: { value, label, createdAt,
updatedAt } } }` (`secrets.cjs`). Any config field anywhere in the app
(MCP env/headers, OpenAPI auth, ...) may contain a `{secret:NAME}` token;
`secrets.resolve()` swaps it in recursively (strings, arrays, objects) at
the point of use and the resolved value is **never written back** to the
record — this is what makes a workspace folder safe to copy/export/sync
without leaking credentials.

### 1.9 Error and timeout handling

- **Connect-time failure** (`connect()` catch block, `index.cjs:276-303`):
  captures the transport's stderr tail (if any and not already in the
  error message), tears down the half-open transport, deletes the `live`
  entry, schedules a retry, logs to the shared `failures.cjs` log
  (`kind:"mcp", tool, error, meta:{serverId, phase:"connect", transport}`),
  and persists `{status:"failed", error, toolCount:0}` to the record. A
  broken server **never throws into a chat turn** — this is stated as the
  file's central design rule.
- **Per-request timeout**: 60s default (`REQUEST_TIMEOUT_MS`), extendable
  per-server via record `timeoutMs`, and extendable per-call via MCP
  progress notifications (heartbeat, §1.2).
- **Mid-session death**: `transport.onClose(error)` rejects every pending
  request with the close reason, marks `"failed"`, schedules retry.
- **IPC-level errors**: every `ipcMain.handle` channel is wrapped by
  `handle()` (`index.cjs:106-114`) which never lets a handler throw across
  IPC — it returns `{ ok:false, error: message }` instead of rejecting the
  promise, so the renderer always gets a typed result to branch on.
- **Connection test** (`mcp:test` IPC, line 796-826): connects using
  possibly-unsaved input, captures tools + stderr + latency, then always
  disconnects again and re-triggers the supervisor — proving a config works
  without leaving an extra connection running.

### 1.10 IPC surface (renderer ↔ main)

All channels wrapped by `handle()` (return `{ok,data}` or `{ok:false,error}`):

| Channel | Args | Purpose |
|---|---|---|
| `mcp:list` | — | Merged view: stored records + live status/error/toolCount/retryAt |
| `mcp:add` | `input` | Create + immediately attempt connect |
| `mcp:update` | `id, changes` | Patch record, disconnect (config changed), clear suppression/backoff, reconnect |
| `mcp:remove` | `id` | Disconnect + delete record |
| `mcp:connect` | `id` | Clear suppression/backoff, `reconnect()` |
| `mcp:disconnect` | `id` | Disconnect, add to `suppressed` (sticky until reconnect/edit), mark idle |
| `mcp:test` | `input` (id string or inline config) | One-shot connect→inspect→disconnect, returns `{ok,error,latencyMs,serverInfo,tools,stderr}` |
| `mcp:status` | — | Live `status()` array for every connected/attempted server |
| `mcp:tools` | `id` | The live tool list of one connected server (throws if not connected/no tools) |

Push event: `mcp:event` sent via `sendToWindow(win, "mcp:event", {type:"server", id})`
whenever a server's live state changes (connect, disconnect, tools
list-changed, unexpected close) — this is what keeps the settings screen
live without polling.

---

## 2. OpenAPI

Files: `openapi/index.cjs` (684 lines — import, storage, calling, IPC),
`openapi/spec.cjs` (1397 lines — pure: hand-rolled YAML subset parser, $ref
resolution, operation→tool conversion, request building, response
formatting).

### 2.1 Ingestion (URL / file / pasted text)

`importSpec(workspace, {name, text, url, baseUrl, serverVariables})`
(`index.cjs:237-321`):
1. Source text comes from `text` verbatim, or fetched from `url` via
   `fetchSpecText()` (line 136-163): GET with
   `Accept: application/json, application/yaml, text/yaml, text/plain`,
   `redirect: "error"` (same anti-exfiltration rule as MCP HTTP), a 30s
   abort timeout (`IMPORT_TIMEOUT_MS`), and a capped read via
   `readCapped()` (line 95-122, streams the body and aborts once it exceeds
   `MAX_SPEC_BYTES = 8 * 1024 * 1024`, i.e. 8 MB) — a truncated fetch throws
   rather than importing a partial document.
2. `spec.parseSpec(source)` — JSON or YAML, auto-detected by a leading `{`
   (§2.2 below covers the parser). Validates: not Swagger 2.0 (checked via
   `document.swagger && !document.openapi`, explicit rejection message
   telling the user to convert first), must have `openapi` starting with
   `"3"`, must have a `paths` object.
3. `spec.resolveRefs(parsed)` — inline local `$ref`s (§2.3), collecting
   warning strings.
4. `spec.listOperations(document)` (line 910-955) — flattens every
   `path × method` pair into an operation record (§2.4); throws if the spec
   describes zero operations.
5. `spec.callability(operation)` filters out operations whose **required**
   body needs a content type Inertia cannot build (multipart, or an
   unrecognized type) — these are **never offered as tools** and are
   listed in the import warnings instead of silently failing every call.
6. Base URL resolution (`guessBaseUrl()`, line 184-203): first
   `document.servers[0]` template is expanded with `spec.resolveServerUrl()`
   using any `serverVariables` the caller supplied; if the resolved URL
   isn't absolute (`^https?://`) it's resolved against the *origin* of the
   source `url` (so a relative `servers: ["/v1"]` works when imported from
   a known host). An explicit `baseUrl` argument always wins over the guess.
7. The **raw ref-resolved document is written to disk** as JSON at
   `plugins/openapi/specs/<id>.json` (`SPEC_DIR`, line 31) — this is the
   copy every later tool build reads; the *original* text is not kept.
8. The import record is written with **every operation disabled by
   default except GET/HEAD** (line 301-304) — "reads on, writes off": a
   30-operation spec does not hand a model 30 ways to POST to a production
   API just because they were in the file.

### 2.2 YAML parsing

`spec.cjs:20-448` — a **hand-rolled recursive-descent YAML subset parser**,
explicitly not a general YAML implementation (no dependency, per the file
header: "a YAML library is a large tree for a format we only need to
read"). Supports: nested block mappings/sequences at any depth, sequences
at the parent key's own indentation (`- item` under a key at the same
column — the common hand-written-OpenAPI style), inline mappings after a
dash (`- name: id`), plain/single/double-quoted scalars, `null`/`true`/
`false`/numbers, block scalars (`|`/`>` with `-`/`+` chomping indicators),
flow collections (`[a, b]`, `{a: 1}`), comments, and a single leading `---`
document marker.

**Explicitly refused, loudly** (raises a descriptive error naming the exact
line and its trimmed content, capped at 80 chars, plus
`"Convert it to JSON and import that instead."`): anchors/aliases (`&x`/
`*x`), merge keys (`<<:`), tags (`!!str`), directives (`%YAML`), multiple
documents, tabs for indentation. Rationale stated directly in the source: a
spec silently parsed wrong becomes tools that call the wrong endpoint with
the wrong arguments, discovered only when an agent does it — refusing is
safer than guessing.

**Rust design note (flagged as hard)**: porting this exactly means either
(a) reimplementing this same hand-rolled subset parser in Rust line-by-line
(most faithful, but a chunk of bespoke parsing logic to maintain), or
(b) using a real YAML crate (`serde_yaml`/`yaml-rust2`) and **deliberately
rejecting** anchors/aliases/merge-keys/tags/multi-doc/tabs post-parse to
preserve the "fail loud rather than guess" behavior and the exact class of
errors surfaced to the user. Option (b) is recommended — less code to
maintain — but note the original's line-number+excerpt error messages are
a deliberate UX feature (`yamlError`/`yamlExcerpt`, lines 56-77) and are
worth reproducing even against a real parser (most YAML crates report a
line/column on parse failure that can be reused for this purpose, but you
lose it for the "we recognize this construct and refuse it" cases, which
need to be pattern-matched over the parsed AST instead of caught as parse
errors).

### 2.3 `$ref` resolution

`spec.resolveRefs(document)` (`spec.cjs:768-805`), independent from and
structurally identical to the MCP-side `resolveSchemaRefs`
(`protocol.cjs:332-370`) — the two are separate implementations of the same
idea in two different files, not shared code.

- Only **local** refs (`#/...`) are resolved, via `pointer(document, ref)`
  (line 741-752) — a straight JSON Pointer walk with `~1`→`/`, `~0`→`~`
  unescaping.
- Cycle-safety: a ref already on the current resolution path collapses to
  `{ type:"object", description:"recursive" }`.
- External refs (anything not starting with `#/`) are **never fetched** —
  become `{}` plus a warning string `"Dropped an external reference to
  <ref>; only local $refs are resolved."` (no network call at import time
  to an unnamed host — same rule as MCP).
- A ref pointing at nothing in the document: `{}` + a warning.
- Sibling keys next to a `$ref` (a description/example added at the use
  site) win over the target's own and are merged in after resolution.
- **No depth cap here** (unlike the MCP-side `MAX_SCHEMA_DEPTH = 12`) —
  cycle detection is the only guard against unbounded recursion; this is a
  latent inconsistency worth normalizing in the port (recommend adding the
  same depth cap for defense-in-depth).

### 2.4 Operations → tools

`listOperations(document)` (`spec.cjs:910-955`): iterates
`document.paths`, for each of the 8 `METHODS`
(`get,put,post,delete,options,head,patch,trace`) present on a path item,
merges path-level `parameters` with operation-level ones (operation's own
definition of the same `name`+`in` wins), and builds:

```js
{
  operationId,     // author's own operationId, sanitized; else synthesized
  method,          // UPPERCASE
  path,
  summary, description,
  parameters,      // [{name, in, required, schema, description}]
  requestBody,     // pickBody() result, or null
  security,        // operation.security ?? document.security ?? null
  tags,
}
```

**Naming** (`synthesiseOperationId`, line 824-832): when a spec has no
`operationId`, one is generated as `<method>_<path-with-braces-as-by_X>` —
e.g. `GET /users/{id}` → `get_users_by_id` (readable, because it becomes
the permission-rule target and the tool id). Collisions get a numeric
suffix (`_2`, `_3`, ...).

**Tool id** (`toolId(prefix, operationId, taken)`, line 1152-1174):
`<sanitized-import-name>_<operationId>`, truncated to
`MAX_TOOL_ID_CHARS = 64` (the common provider floor). If truncation causes
a collision within the `taken` set (long generated operationIds sharing a
common prefix — common with auto-generated specs), a deterministic FNV-1a
hash (`shortHash`, line 1123-1130, base-36) of the *full* untruncated id is
spliced onto the end so the collision becomes an ugly-but-stable name
rather than a silently shadowed tool. The `taken` set is **shared across
every import in the workspace**, not just within one spec — two imported
APIs cannot claim the same tool id.

**Parameter schema derivation** (`toolParameters`, line 1047-1081): one
flat JSON-Schema object.
- Path/query/header parameters go at the top level under their own name
  (`relaxSchema()`'d, description filled from the parameter's own
  `description` or a synthesized `"The <name> <in> parameter."`).
  - `in: cookie` parameters are **never offered** (a session cookie is not
    something a model can invent).
  - A header parameter already supplied by the chosen auth scheme
    (`authCovered()`, line 1029-1037 — e.g. `Authorization` when
    auth is bearer/basic) is **dropped from the schema** so the model
    can't double-set it.
  - `required` if the parameter says so, or unconditionally if `in:path`.
- The request body, when buildable, goes under a single `body` property
  (never merged into the top level — its keys are the API's own and would
  collide with parameter names), required if `requestBody.required`.
- A body that is **not buildable is left out of the schema entirely**
  rather than offered and then refused at call time.

**Schema relaxation for strict providers** (`relaxSchema()`, line
1015-1027 — a distinct, OpenAPI-specific implementation from the
Composio/MCP ones, but conceptually identical): drops
`additionalProperties: false` (a closed object turns a model's improvised
field into a hard tool-call failure instead of a correctable API 400),
drops any `format` value not in the fixed `KNOWN_FORMATS` set (line
967-992 — the standard JSON Schema/OpenAPI format vocabulary; vendor
private annotations like `format: "decimal"` are stripped because a
strict-mode provider validates format and rejects the *entire tool list*
over one unrecognized value on one tool), drops `nullable` (OpenAPI 3.0
spelling, not JSON Schema's — leaving the bare type is more permissive
than either).

**Description** (`describe()`, line 1097-1105): `summary + "\n\n" +
description`, capped to `MAX_DESCRIPTION_CHARS = 1024`, with `"<METHOD>
<path>"` always appended as the last line — stated as often being "the
clearest single statement of what the thing does" even against a vague or
absent summary.

**Body-format support** (`bodyStyle()`, line 845-852; `pickBody()`, line
865-892): only `json` (any `*/json` content type), `form`
(`application/x-www-form-urlencoded`), and `text` (`text/*` or `*/xml`,
sent through as-is) are buildable. `multipart/*` and anything else are
marked `unsupported` with a human reason string; if the request body is
**required** and unsupported, `callability()` (line 1091-1095) drops the
whole operation from import (§2.1 step 5); if it's optional-and-unsupported
the operation still imports but the body is left out of its schema.
When several content types are offered, preference order is
`BODY_PREFERENCE = ["json","form","text"]` (line 855).

### 2.5 Auth handling

Import-time classification (`describeSecurityScheme()`, `spec.cjs:595-681`)
maps each declared `components.securitySchemes` entry to one of the four
things this app can actually construct, or explains why not:

| OpenAPI scheme | Supported? | `authType` |
|---|---|---|
| `type: http, scheme: bearer` | yes | `bearer` |
| `type: http, scheme: basic` | yes | `basic` |
| `type: apiKey, in: header` | yes | `header` |
| `type: apiKey, in: query` | yes | `query` |
| `type: apiKey, in: cookie` | no | — ("Inertia does not hold cookies for an imported API") |
| `type: http`, other schemes | no | — |
| `type: oauth2` | no | — ("Get a token from the vendor yourself... or connect through Composio instead") |
| `type: openIdConnect` | no | — |
| `type: mutualTLS` | no | — ("cannot present a client certificate") |
| anything else | no | — |

`describeSecurity(document)` also determines which schemes are actually
**referenced** (via `document.security` or any per-operation `.security`)
vs merely declared-but-unused, and `securityWarnings()` (line 730-737)
raises an import warning **only** when the referenced set has zero
supported options (a spec offering OAuth2 *and* an API key is fine; one
offering only OAuth2 gets the warning).

**Storage**: the record's `auth` field (`{type:"none"|"bearer"|"basic"|
"header"|"query", token/username/password/name/value: "{secret:NAME}"}`)
is written through **as given, secret-refs and all** — the renderer is
responsible for never storing a resolved secret value here; the backend
never resolves-then-persists.

**At call time** (`buildRequest()`, `spec.cjs:1319-1338`), resolved auth
(`resolveAuth()`, `index.cjs:356-359`, via `secrets.resolve()`) is applied:
`bearer` → `Authorization: Bearer <token>`; `header` → the named header;
`query` → appended to the query string; `basic` → `Authorization: Basic
<base64(user:pass)>`; `none`/unrecognized → sent bare.

### 2.6 Request construction and response mapping

`buildRequest(operation, args, {baseUrl, auth})` (`spec.cjs:1264-1347`,
pure — no fetch):
- Path parameters: substituted with `encodeURIComponent` (a raw value
  could contain `/` and silently address a different route); missing
  **required** path param throws immediately (`Missing path parameter
  \`x\` for GET /path.`).
- Query parameters: array values repeat the key (`?tag=a&tag=b`) —
  described as "the default OpenAPI style... the one every server
  understands," not comma-joined.
  Undefined/null values are skipped, not sent as empty.
- Header parameters: set verbatim.
- Body: `json` → `JSON.stringify` (or passthrough if the model already
  sent a string); `form` → `encodeFormBody()` (line 1242-1255 — nested
  objects have no universally-agreed form encoding, so they're serialized
  as JSON under their own key as a documented best-effort guess); an
  **unsupported body throws** rather than being sent malformed (refused by
  name instead of producing a confusing API-side 400 the model can't
  interpret as its own mistake).
- Auth applied last (§2.5).
- URL = `joinUrl(baseUrl, path)` (strips trailing `/` from base, ensures
  leading `/` on path) `+ "?" + querystring` if any.

`callOperation()` (`index.cjs:370-430`) executes the built request:
- Guards: no `baseUrl` set → throws immediately; `baseUrl` still containing
  `{}` (unresolved server-variable template) → throws with a pointer to
  Settings.
- `fetch(url, {method, headers, body, signal, redirect:"error"})` — same
  no-redirect rule as MCP HTTP and the spec fetch.
- `CALL_TIMEOUT_MS = 60000` (line 41) — an API call budget separate from
  and larger than the 30s spec-fetch timeout, "some of these do real
  work."
- Response body capped at `MAX_RESPONSE_BYTES = 1MB` via the same
  `readCapped()` streaming-cap pattern as the spec fetch.
- **A non-2xx response is not an error** — it's returned as a normal tool
  result via `formatResponse()` (`spec.cjs:1359-1371`, pretty-prints JSON
  bodies, leaves other bodies as-is, prefixes with
  `"<status> <statusText> <method> <path>"`). Only failure to reach the
  server at all (`describeNetworkError()`, `index.cjs:124-134` — maps
  `ECONNREFUSED`/`ENOTFOUND`/`ETIMEDOUT`/abort to readable sentences) is a
  thrown tool error. This is the same philosophy as MCP's `isError`
  handling: the API's own words are more actionable to the model than a
  generic "request failed."

### 2.7 Spec validation and error reporting

- Empty document, non-object top level, Swagger 2.0, missing/wrong
  `openapi` version, missing `paths`, zero operations, all-uncallable
  operations — each throws a specific, human-readable `Error` at
  `importSpec()` time (§2.1), never a generic parse failure.
- Non-fatal issues accumulate as `warnings: string[]` on the record:
  dropped external/dangling `$ref`s, skipped uncallable operations (with
  per-operation reasons), base-URL template placeholders, unsupported auth
  schemes. These are surfaced in the settings UI, not swallowed.
- `openapi:test` IPC (line 660-671) calls one operation with user-typed
  args immediately, surfacing the API's actual response in the settings
  screen rather than only discovering a bad base URL mid-agent-turn.

### 2.8 Tool selection / capping and IPC

`MAX_TOOLS = 120` total across **all imports combined**
(`index.cjs:58`) — every tool schema is sent to the model on every turn, so
an unbounded import would burn the entire context describing an API before
the conversation is read. `selectOperations()` (line 450-458) ranks:
explicitly-enabled operations first (weight 0), then GETs (weight 1, "the
least dangerous thing to have available"), then everything else (weight
2), stable by original order within a weight tier, then slices to the
remaining room. `openapiTools()` (line 467-511) walks enabled records in
order, and **one broken import (e.g. its stored spec file deleted by hand)
is skipped silently** rather than taking down the rest.

**On-disk shape**:
- Record: `plugins/openapi/<id>.json` — `{id, name, description, specFile,
  baseUrl, serverVariables, sourceUrl, auth, enabled, operations: [{id,
  enabled}], warnings, createdAt, updatedAt}`.
- Spec: `plugins/openapi/specs/<id>.json` — the full ref-resolved document,
  as JSON regardless of original format.

**IPC surface**: `openapi:list`, `openapi:import`, `openapi:details` (id)
→ `{servers, security, warnings, serverVariables}` (re-derived from the
stored document on every call, not cached on the record, so it stays
accurate after edits), `openapi:remove`, `openapi:update` (name/description/
baseUrl/enabled/serverVariables/auth — changing `serverVariables` without
an explicit `baseUrl` re-derives the base URL from the spec), `openapi:
operations` (id) → per-operation `{id, method, path, summary, tags,
callable, reason, enabled}` for the settings checklist, `openapi:
set-operations` (id, rows), `openapi:test` (id, operationId, args). Push
event `openapi:event` on import/removed/updated.

---

## 3. Composio

Files: `composio/api.cjs` (841 lines — pure HTTP client + normalization,
no Electron/workspace dependency), `composio/index.cjs` (707 lines —
records, IPC, tool building, connection lifecycle), `composio/logos.cjs`
(161 lines — fetches a toolkit logo URL and returns it as a data URI,
because the renderer's CSP forbids remote `<img src>`).

### 3.1 API surface used

Base URL: `https://backend.composio.dev` (`DEFAULT_BASE_URL`,
`api.cjs:24`), configurable. Auth: every request carries header
`x-api-key: <key>` (`request()`, line 124-174) — **no SDK**, eight
endpoints implemented directly over `fetch`:

| Endpoint | Method | Used for |
|---|---|---|
| `/api/v3/toolkits` | GET | Full catalogue (paged) |
| `/api/v3/toolkits/{slug}` | GET | One toolkit's name/logo |
| `/api/v3/auth_configs` | GET, POST | Find/create the OAuth-client record for a toolkit |
| `/api/v3/tools` | GET | Every operation a toolkit exposes (paged) |
| `/api/v3/tools/execute/{slug}` | POST | Run one tool |
| `/api/v3/connected_accounts/link` | POST | Start an OAuth handshake |
| `/api/v3/connected_accounts/{id}` | GET | Poll one connection's status |
| `/api/v3/connected_accounts` | GET | Every connected account (paged), for bulk sync |
| `/api/v3/connected_accounts/{id}` | DELETE | Revoke |

Every request: 30s abort timeout (`REQUEST_TIMEOUT_MS`), `redirect:
"error"` (the `x-api-key` header must never be replayed to a redirect
target), a non-2xx is turned into a human sentence via `describeFailure()`
(line 79-106 — reads the JSON error body's `error.message`/`message`/
`error`, and prefixes with a status-coded headline: 401 "Composio rejected
the API key.", 403 "...not allowed to do that.", 404 "Composio has nothing
at <url>.", 429 "Rate limited...", 500/502/503 mapped similarly). Network
failures (DNS, ECONNREFUSED, timeout) go through `describeNetworkError()`
(line 109-118), same pattern as MCP/OpenAPI.

**Pagination**: `collectPages(fetchPage, {max})` (line 187-206) — a shared
generic cursor walker used by toolkits/tools/authConfigs/connectedAccounts.
Treats a **repeated cursor as end-of-list** rather than trusting the
server (guards against a server that echoes back the cursor it was given,
which previously caused unbounded re-fetching — noted as an actual
historical bug in the comments). Hard ceilings: `MAX_TOOLKITS = 3000`,
`MAX_AUTH_CONFIGS = 1000`, `MAX_TOOLS_PER_TOOLKIT = 500`.

### 3.2 How connections/accounts are established and stored

Composio's model: a **toolkit** (an app, e.g. `gmail`) needs an **auth
config** (Composio's record of "how do we log into this app" — usually
its own managed OAuth client) to create a **connected account** (one
user's actual credential).

`getAuthConfig(key, toolkitSlug)` (`api.cjs:491-512`): looks for an
existing auth config for the toolkit first (filtered by `toolkit_slug`,
**not** the unfiltered `toolkit` param — the comment notes this was an
actual bug: an unfiltered list returned *any* app's config, so connecting
Gmail sometimes opened Instagram's consent page). If none exists, creates
one: `POST /api/v3/auth_configs` with `{toolkit:{slug}, auth_config:{type:
"use_composio_managed_auth"}}`. `pickAuthConfig()` (line 529-536) prefers
a Composio-managed one but falls back to a user-made one when that's all
there is (the mechanism that lets an app Composio doesn't manage OAuth for
still be connectable, once the user has manually created an auth config
for it in the Composio dashboard).

`connect(toolkitSlug)` (`index.cjs:408-442`):
1. `getAuthConfig()`.
2. Generate a **per-connection** Composio user id:
   `newComposioUserId(toolkitSlug)` = `"<slug>-<16 hex chars>"`
   (`crypto.randomBytes(8)`) — scoping matters because Composio routes
   execution by user id, and two Gmail accounts sharing one id would be
   indistinguishable at call time.
3. `linkAccount(key, {authConfigId, userId})` — `POST
   /api/v3/connected_accounts/link` with `{auth_config_id, user_id}`,
   returns `{accountId, redirectUrl, status}` (every historically-seen
   field spelling for id/url is read defensively — `id`/
   `connected_account_id`/`connectedAccountId`/`connectionId`, `redirect_url`/
   `redirectUrl`/`redirect_uri`).
4. Fetch the toolkit's display name/logo (best-effort, non-fatal on
   failure).
5. Write a `plugins.composio` record with `status: link.status` (typically
   `"INITIATED"`) and `redirectUrl` returned to the caller — **nothing is
   connected yet**; the user must open `redirectUrl` and complete the
   vendor's OAuth consent screen.

Polling: `composio:status` IPC / `status(id)` (`index.cjs:476-498`) —
`GET /api/v3/connected_accounts/{id}`, writes the returned `status`/`label`
back to the record; the renderer polls this until status leaves
`"INITIATED"`. Once it becomes `"ACTIVE"` with no label yet, an **identity
lookup** runs (§3.3).

Account status set considered "dead" (`DEAD_ACCOUNT_STATUSES`, line 610):
`EXPIRED, FAILED, INACTIVE, DELETED, REVOKED` — deliberately excludes
`INITIATED` (mid-handshake, not yet failed).

**Reconnect** (`composio:reconnect`, `index.cjs:614-647`): re-links using
the **same stored `composioUserId`** and keeps the record's id (and
therefore its `enabledTools` selection) — old account is deleted remotely
only *after* the new link succeeds (never leaves the user with nothing if
the new link fails).

**Bulk sync** (`composio:sync`, line 570-603): one paged
`listConnectedAccounts()` read reconciles every stored connection's status
in a single round trip rather than N — an account Composio has no record of
at all is marked `"MISSING"` (not silently deleted; the user's own row is
preserved).

### 3.3 Account identity ("whose account is this")

Composio's connected-account payload carries OAuth tokens but not a
human-readable identity. `IDENTITY` (`index.cjs:178-184`) is a small
**hardcoded per-toolkit table** — currently just `gmail`, calling
`GMAIL_GET_PROFILE` with `{user_id:"me"}` and reading
`data.response_data.emailAddress`. `labelled()` (line 211-225) runs this
lookup **at most once per record per process run**
(`identityAsked: Set<recordId>`), writes the result to `label` on the
record, and is invoked lazily both from the polling path and from
`composioTools()` right before building tools for that connection — a
toolkit not in `IDENTITY` simply keeps an empty label forever.

### 3.4 How Composio actions are discovered and turned into tools

`toolkitTools(key, toolkitSlug)` (`index.cjs:246-252`): `listTools()`
(`api.cjs:446-478`, paged to exhaustion, `TOOL_PAGE_LIMIT=100`,
`MAX_TOOLS_PER_TOOLKIT=500`), cached per-toolkit-slug in a module-level
`Map` (`toolCache`), invalidated on connection changes.

`selectTools(all, enabledTools)` (`index.cjs:305-313`): if the connection
record's `enabledTools[]` is empty, **all** of the toolkit's tools are
candidates (default-on, unlike OpenAPI's default-off — the toolkit was
already an explicit, deliberate connect action); otherwise filtered to the
named slugs. `importantFirst()` (line 300-303) sorts tools tagged
`"important"` by Composio first. Per-toolkit cap
`MAX_TOOLS_PER_TOOLKIT = 40`, global cap across all connected apps
`MAX_TOOLS_TOTAL = 96` (both in `index.cjs:35-36` — smaller than MCP/
OpenAPI's ceilings, "a model given every operation of every connected app
stops choosing well").

`api.toTool(toolkitSlug, tool, execute)` (`api.cjs:789-808`):
- id = `tool.slug.toLowerCase()` — Composio slugs are `SCREAMING_SNAKE`;
  lower-cased for the model because every other tool id in the app is
  lowercase and a model that sees both cases in one tool list guesses
  wrong about which convention a given tool follows. The **original-case
  slug is preserved** in `metadata.composioSlug` because Composio's
  execute endpoint only answers to its own spelling.
- `parametersOf()` (line 759-765): Composio may return either a full JSON
  Schema or a bare `{propertyName: schema}` bag — wrapped into
  `{type:"object", properties}` if it's not already typed.
- `relaxSchema()` (line 745-756, a third independent implementation of the
  same idea as MCP's and OpenAPI's): strips `required` arrays and
  `additionalProperties:false` at every depth — same rationale, a strict
  provider rejecting the whole tool list over a schema written for a
  validator rather than a model.
- Description capped to `MAX_DESCRIPTION_CHARS = 1024`.
- **Permission key**: `{ key:"composio", target: () => toolkitSlug }` —
  scoped to the **whole toolkit**, not per-operation (unlike MCP/OpenAPI)
  — "a rule can read `allow composio gmail` and mean it."

### 3.5 Execution flow

`composioTools(workspace)` (`index.cjs:322-396`), called on the turn path:
1. No API key → returns `[]` immediately (not an error — an app with no
   Composio key configured simply offers none of these tools).
2. Filters connection records to `enabled !== false` and
   `status === "ACTIVE"`.
3. For each, `labelled()` (fills identity label, §3.3, at most once).
4. `toolkitTools()` for that slug — **one broken toolkit is skipped
   silently** (no log — "this runs before every model turn, and a toolkit
   that is down would otherwise fill the log with the same line"),
   consistent with MCP/OpenAPI's per-source fault isolation.
5. For each selected tool, wraps `api.execute(key, tool.slug,
   {arguments_, connectedAccountId: row.accountId, userId:
   row.composioUserId}, {signal})`.

`execute()` (`api.cjs:714-730`, `POST /api/v3/tools/execute/{slug}`)
returns `{ok, data, error}` — Composio's own `successful` flag, not an
HTTP status, decides success. **A failed execution is a normal outcome**
(bad filter, unshared sheet, closed issue), returned to the model as
readable text, not thrown, **unless** it's an auth failure.

**Auth-failure detection and remedy** (`isAuthFailure()`, line 690-694,
matched against `AUTH_FAILURE_SIGNALS`, a ~19-entry substring list —
`invalid_grant`, `expired`, `unauthorized`, `revoked`, `401`, `403`, etc.,
line 663-688): Composio doesn't flag "this is an auth problem" structurally
— every vendor words its own refusal differently — so this is a
deliberately loose substring match, explicitly justified as "the cost of a
false positive is a slightly wrong sentence pointing at the reconnect
button, and the cost of a false negative is the status quo." On a match,
`markExpired()` (`index.cjs:288-297`) writes `status:"EXPIRED"` to the
record (so `composioTools()` stops offering that toolkit on the *next*
turn) and the tool call fails with `authFailureMessage()` (line 269-277):
*"`<app>` is no longer authenticated with Composio, so this call cannot
succeed and retrying it will not help. Reconnect `<app>` under Settings,
Integrations, Composio."* — worded specifically so the model stops
retrying with different arguments (the historical failure mode being
described: the model reads `invalid_grant`, assumes it passed a bad
argument, burns three turns rewriting arguments that were never the
problem).

Successful result: `{ title: "<connection name>: <tool name>", output:
truncateResult(result.data), metadata: {toolkitSlug, composioSlug} }`.
`truncateResult()` (`api.cjs:775-780`, `MAX_RESULT_CHARS = 16000`) —
**re-wraps** an oversized result as `JSON.stringify({_truncated:true,
result_preview: <slice>})` rather than naively slicing the serialized JSON
(which would hand the model invalid, mid-token JSON to try to parse).

### 3.6 Catalogue caching

`catalogue({search, category, refresh})` (`index.cjs:81-104`): an unfiltered
listing is cached to disk at `cache/composio-catalogue.json`
(`CATALOGUE` document key, `layout.cjs:102`) and served from there when
younger than `CATALOGUE_MAX_AGE_MS = 6 hours` — described as existing only
"so a machine left running for a week eventually notices the world moved";
the intended refresh mechanism is the user pressing Refresh (`refresh:
true` invalidates both the disk cache and the in-memory tool cache). A
search/category-narrowed query always hits the live API (never
cached/stored) since search happens client-side over the full list anyway.

### 3.7 On-disk shapes

**Connection record** — `plugins/composio/<id>.json`:
```jsonc
{
  "id": "...",
  "toolkitSlug": "gmail",
  "name": "Gmail",
  "logo": "https://...",
  "accountId": "ca_...",        // Composio's connected_account id
  "composioUserId": "gmail-a1b2c3d4e5f6...",
  "status": "ACTIVE",           // INITIATED|ACTIVE|EXPIRED|FAILED|INACTIVE|DELETED|REVOKED|MISSING
  "label": "someone@gmail.com", // filled lazily via IDENTITY lookup
  "enabled": true,
  "enabledTools": [],           // [] = every tool in the toolkit
  "createdAt": "...", "updatedAt": "..."
}
```

**API key**: stored as a named secret (default name `COMPOSIO_API_KEY`,
overridable via `settings/app.json`'s `composioKeySecret` field), resolved
through the same central `secrets.cjs` module used by MCP/OpenAPI — never
stored on the connection record itself.

**Catalogue cache**: `cache/composio-catalogue.json` — `{items:
[NormalisedToolkit], fetchedAt: <ms>, nextCursor: null}`.

### 3.8 IPC surface

`composio:configured` (bool — is a key present), `composio:logo` (url →
data URI), `composio:toolkits` (query → catalogue, cache-aware),
`composio:tools` (toolkitSlug → live tool list), `composio:connections`
(list all records), `composio:refresh` (drop tool cache, optionally
scoped to one toolkit), `composio:sync` (bulk reconcile against Composio,
§3.2), `composio:reconnect` (id), `composio:connect` (toolkitSlug →
`{id, redirectUrl, status}`), `composio:status` (id, poll), `composio:
disconnect` (id — best-effort remote revoke, local row always removed),
`composio:permissions` (id → `{enabled, enabledTools}`), `composio:
set-permissions` (id, `{enabled, enabledTools}`). Push event
`composio:event` on catalogue refresh and any connection-state change.

**App-internal read**: `connectedApps(workspace)` (`index.cjs:232-242`) —
a cheap, key-less list of `{id, toolkitSlug, name, label}` for currently
active connections, used to describe what's connected in the system
prompt without touching the network.

---

## 4. Cross-cutting comparison

| | MCP | OpenAPI | Composio |
|---|---|---|---|
| Transport | stdio (child process) / Streamable HTTP | plain HTTPS (fetch) | plain HTTPS (fetch) |
| Discovery | live JSON-RPC `tools/list` against a running connection | parsed once at import, cached on disk as resolved JSON | live paged REST catalogue, disk-cached 6h |
| Tool cap | none explicit (bounded only by what's connected) | 120 total | 96 total, 40/toolkit |
| Default enablement | all tools of a connected server | GET/HEAD only, others opt-in | all tools of a connected toolkit |
| Permission granularity | per-server-tool (`server/tool`) or per-server (`server/*`) | per-operation (`METHOD path`) or per-method (`METHOD *`) | per-toolkit only |
| Credential storage | `{secret:NAME}` refs in env/headers | `{secret:NAME}` refs in `auth` block | Composio holds the token; app stores only the Composio API key + account id |
| Schema relaxation | inline local `$ref`, strip validator metadata, keep `additionalProperties:false` | inline local `$ref`, strip unknown `format`/`nullable`/`additionalProperties:false` | strip `required`/`additionalProperties:false` |
| Non-2xx / tool-level failure | `result.isError` → thrown `ToolError` with flattened content | any HTTP status → returned as text (never thrown) | `successful:false` → returned as text unless auth failure |
| Auth-expiry detection | n/a (connection-level failure only) | n/a (no auth-refresh concept — user swaps the secret) | substring-matched vendor error → marks record EXPIRED, blocks re-offering, tells model to stop retrying |
| Per-source fault isolation | one bad server ⇒ its tools drop, `mcpTools()` continues | one bad import (missing spec file) ⇒ skipped, others continue | one bad toolkit ⇒ skipped, others continue |
| Redirect policy | refused everywhere (`redirect:"error"`) | refused everywhere | refused everywhere |

All three, plus the spec-fetch path, independently reimplement the **same
three ideas**: (1) resolve local `$ref`s only, never fetch external ones;
(2) relax a schema so a strict-function-calling provider won't reject the
whole request over one field; (3) never let a per-item failure propagate
past its own source. This repetition is a strong signal for the Rust port:
these three belong in one shared module (`schema_relax.rs`,
`ref_resolve.rs`) called from all three provider crates/modules rather than
copied a fourth time.

---

## 5. Rust design notes

### 5.1 The `ToolProvider` seam

See §0 for the trait shape. Concretely, three implementations:

```
crate tools_mcp      -> MCPProvider   (owns the live connection map + supervisor task)
crate tools_openapi  -> OpenApiProvider (owns the tool-list cache keyed by workspace)
crate tools_composio -> ComposioProvider (owns the per-toolkit tool cache + catalogue cache)
```

Registry composition mirrors `registry.cjs::fromSources` exactly: iterate
providers, `tokio::try_join!`-independent (each provider call wrapped in
its own error handler so one failing provider doesn't fail the others —
use `Result` per provider and collect into a `problems: HashMap<&str,
String>` map, not `?`), concatenate, **sort by `tool.id`** for prompt-cache
locality (this ordering requirement is easy to silently drop in a rewrite
and would quietly regress prompt caching — call it out in code review).

### 5.2 MCP stdio process management — the hard part

This is the highest-risk area of the whole port. Node's `child_process`
gives you, almost for free: unref'able timers, straightforward
pipe-based stdio, a `detached` flag plus process-group signaling on POSIX,
and `spawn` failure vs. runtime failure as distinguishable async events.
In Rust with `tokio::process::Command`:

- **Process groups / tree-killing**: Rust has no direct equivalent of
  `detached: true` + `process.kill(-pid)`. On Unix, use
  `std::os::unix::process::CommandExt::process_group(0)` (stable since
  Rust 1.64) to put the child in its own group, then send the signal to
  `-pid` via the `nix` crate (`nix::sys::signal::kill(Pid::from_raw(-pid),
  Signal::SIGTERM)`). On Windows, there is no built-in equivalent of
  `taskkill /T` — either shell out to `taskkill.exe /pid <pid> /T /F`
  exactly as the original does, or create the child in a Job Object
  (`CreateJobObject` + `AssignProcessToJobObject` +
  `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) via `windows-rs`/`win32job` crate —
  the Job Object approach is more idiomatic Rust and auto-kills the whole
  tree even on ungraceful app exit, which is strictly better than the
  original's explicit-close-only behavior; recommended over shelling out
  to taskkill.
- **Windows shell quoting for `npx`/`uvx`**: `windowsCommandLine()`
  (`transport.cjs:61-67`) must be ported faithfully — this is a real,
  specific bug fix (`shell:true`-style joining breaks on any argument
  containing a space), and Rust's `std::process::Command` has the *same*
  problem (no automatic shell-quoting) if you spawn `cmd.exe /c` directly.
  Port the quoting function verbatim; do not assume a crate handles it —
  `tokio::process::Command` does not special-case Windows batch-file
  spawning either. (Newer Rust std does apply CommandLineToArgvW-style
  escaping automatically when *not* going through a shell, but here you
  need `cmd.exe /c` specifically because `npx.cmd` isn't directly
  executable, so the manual quoting step is still required.)
- **Line-based framing across `stdout`**: straightforward —
  `tokio::io::BufReader` + `.lines()`, or reuse the exact `parseFrames`
  buffering logic against a `BytesMut` if reads arrive as raw chunks
  instead of a line-oriented reader.
- **Stderr ring buffer**: trivial — a `String`/`VecDeque<u8>` capped at
  8000 bytes, appended on each `stderr` chunk.
- **Spawn-timeout race** (pid vs. error within 30s): `tokio::select!` over
  `child.wait()` (or an `id()` check) and a `tokio::time::sleep`.
- **Async correlation table** (`createPeer`'s `pending: Map`): a
  `tokio::sync::Mutex<HashMap<u64, oneshot::Sender<Result<Value,
  RpcError>>>>` plus a monotonic `AtomicU64` id counter. Timeouts as
  per-entry `tokio::time::sleep` tasks that race the oneshot receiver, or a
  single `DelayQueue` (from `tokio-util`) keyed by request id for the
  rearm-on-progress behavior — `DelayQueue::reset` maps directly onto
  `entry.rearm()`.
- **Supervisor loop**: `tokio::time::interval(Duration::from_secs(15))`
  with the same due-set/backoff/suppressed-set logic; run it as a
  dedicated background task owned by `MCPProvider`, spawned once at app
  start, `.abort()`ed on shutdown.

Recommended crates: `tokio` (process, time, sync), `nix` (POSIX signals),
`win32job` or `windows` (Windows job objects), no MCP-specific crate
needed — the wire protocol really is small enough to hand-roll as the
original does, and doing so keeps parity with the exact framing/timeout/
heartbeat behavior documented above rather than inheriting a third-party
SDK's different opinions about retries, defaults, and error shapes.

### 5.3 OpenAPI $ref resolution and YAML — the other hard part

- **JSON Schema `$ref` resolution**: straightforward to port as a
  recursive `serde_json::Value` walker mirroring `resolveSchemaRefs`/
  `resolveRefs` almost line-for-line (`serde_json::Value` maps cleanly onto
  the JS object model used here). Do **not** reach for a full JSON-Schema
  crate (`jsonschema`, `schemars`) for this — the original's ref resolution
  has bespoke behavior (cycle → opaque placeholder, external → `{}` + a
  warning string, sibling-keys-win) that a general-purpose validator
  doesn't implement and isn't meant to.
- **YAML**: see §2.2 — recommend `serde_yaml` (or `yaml-rust2` if lower-
  level control over anchors detection is wanted) plus a post-parse guard
  pass that rejects the same disallowed constructs with equivalent
  line-numbered error messages. A real YAML crate will silently *resolve*
  anchors/aliases rather than refuse them like the original does, so this
  guard pass is not optional — it is required to preserve the "never
  silently misinterpret a spec" guarantee. This is the one area where a
  literal line-by-line port of the hand-rolled parser might actually be
  *less* work than bolting refusal behavior onto a general parser's more
  permissive AST, depending on how much of `yaml-rust2`'s intermediate
  representation exposes anchor usage before resolution.
- **Request building** (`buildRequest`): a pure function, ports trivially.
  Use `reqwest` or `hyper` directly (matching the original's direct-`fetch`
  philosophy over any generated-client approach) with `redirect::Policy::none()`
  to reproduce the mandatory `redirect:"error"` behavior everywhere a
  credential or spec is fetched.

### 5.4 Composio

The lowest-risk of the three — it's a thin REST client with no process
management and no parsing beyond JSON. `reqwest` + `serde` port it
directly; the interesting parts to preserve are the **cursor-repeat-as-end**
pagination guard (`collectPages`), the **auth-failure substring list**
(keep it as a `const &[&str]` exactly as enumerated in §3.5 — this is a
deliberately hand-tuned heuristic, not a general-purpose classifier, and
should not be "improved" during the port without re-validating against
real vendor error strings), and the **per-connection Composio user id**
scoping (`toolkit_slug-<16 hex chars>`, trivially `rand`/`hex` in Rust).

### 5.5 Shared workspace/secrets substrate

All three depend on:
- `collections.cjs`'s one-file-per-record CRUD over a `plugins/<kind>/`
  directory, and its `readDocument`/`writeDocument` for singleton files
  (catalogue cache, settings). Port as a small generic `Collection<T:
  Serialize + DeserializeOwned>` over `tokio::fs`, keyed by the same
  `layout.rs` path table (`plugins/mcp`, `plugins/openapi`,
  `plugins/openapi/specs`, `plugins/composio`, `secrets/secrets.json`,
  `cache/composio-catalogue.json`).
- `secrets.cjs`'s `{secret:NAME}` token substitution — a simple regex
  swap (`Regex::new(r"\{secret:([A-Za-z_][A-Za-z0-9_]*)\}")`) applied
  recursively over a `serde_json::Value`, resolved at the point of use in
  each provider and never re-persisted. This convention is what keeps a
  workspace folder portable/exportable without leaking credentials across
  all three integrations, and must be preserved exactly (same token
  syntax) so existing exported workspaces continue to work against the
  Rust build.

### 5.6 Suggested crate list

| Concern | Crate |
|---|---|
| Async runtime | `tokio` (full: process, time, sync, fs) |
| HTTP client (OpenAPI calls, Composio, MCP HTTP transport, spec fetch) | `reqwest` (rustls-tls, with `redirect::Policy::none()`) |
| SSE parsing (MCP Streamable HTTP) | hand-rolled (mirrors `sseFrames`/`sseEvents`, ~30 lines) or `eventsource-stream` |
| JSON | `serde`, `serde_json` |
| YAML | `serde_yaml` + custom guard pass (§5.3) |
| POSIX process-group signaling | `nix` |
| Windows job objects | `win32job` |
| Regex (secret substitution) | `regex` |
| Random ids (Composio user id) | `rand` |
| FNV hash (OpenAPI tool-id collision suffix) | `fnv` crate, or hand-roll (12 lines, matches `shortHash` exactly) |

### 5.7 Things easy to silently regress in a port — checklist

- Tool list **sort by id** before returning from the registry (prompt-cache locality).
- **First-registered-wins** on MCP tool-id collision (not last).
- `additionalProperties: false` is **kept** in MCP schema sanitizing but
  **stripped** in OpenAPI/Composio relaxation — this asymmetry is
  intentional (MCP servers are typically well-behaved schema authors,
  OpenAPI/Composio schemas are "written for a validator") and easy to
  collapse into one shared function by mistake.
- `redirect: "error"` on **every** outbound fetch that carries a secret or
  fetches a spec/catalogue (MCP HTTP transport, OpenAPI spec-fetch,
  OpenAPI call, Composio requests).
- Never fetch an **external** `$ref` (network call to an unnamed host at
  import/connect time) — both MCP and OpenAPI ref resolvers enforce this
  independently; a shared Rust implementation must not accidentally add
  fetch capability "since it's already async."
- A single record/import/connection failing must never propagate past its
  own source into the combined tool list (`try`/`continue`, not `?`, at
  the per-item level inside each provider's `tools()`).
- Backoff/suppression state (`retries`, `suppressed` in MCP) is
  **process-lifetime only**, never persisted — a relaunch is read as "try
  again now."
- Secrets are resolved at the point of use and **never written back** to
  any record, anywhere.
