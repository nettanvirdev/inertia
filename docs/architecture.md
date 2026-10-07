# Architecture

How Inertia is put together: the Rust workspace, the window, the boundary
between them, and what happens when you send a message.

For the exact list of commands and events, see [backend-api.md](backend-api.md).
For the files on disk, see [workspace.md](workspace.md).

## The shape of it

Inertia is a [Tauri 2](https://tauri.app) app. The window is React 19, built
with Vite and served from `dist/`. Everything else is Rust.

```
src/                     the window (React, plain JSX)
  bridge/                the whole of the boundary to Rust, one module per namespace
  features/              one folder per screen: chat, agents, routines, computers, ...
  shared/                pure logic both halves agree on: permission keys, modes, ...
src-tauri/
  Cargo.toml             Cargo workspace root, and the `inertia` app package
  src/                   the app crate: commands, wiring, app-level tools
  crates/                every piece with real logic in it; none of them know Tauri exists
installer/, uninstaller/ two more small Tauri apps (Windows setup bootstrapper and uninstaller)
scripts/build-setup.mjs  builds the Windows setup end to end
```

## The one rule

**Dependencies point inward, toward `inertia-core`, and nothing in `crates/`
depends on Tauri.**

If a crate under `crates/` needs something only the app has (an `AppHandle`, a
window, an emitter), the app passes it in through a trait or a closure. That is
what lets the same agent loop run headless against mocks in `inertia-devkit`,
and what lets each crate be tested without a window.

## Crates

| Crate | What it does |
|---|---|
| `inertia-core` | Domain types and the traits everything else implements: `Provider`, `Tool`, `ToolProvider`, `ToolRegistry`, `PermissionGate`, `Lifecycle`, `Clock`. Also the pure permission rule engine. |
| `inertia-agent` | The turn loop: build a request, stream it, run tool calls, repeat. Step budget (100 by default), repeated-call guard, prompt assembly, the conversation modes and which tools each withholds, the group-conversation floor, and steering a running turn. Depends only on `inertia-core`. |
| `inertia-provider` | Two wire protocols: Anthropic Messages and OpenAI chat completions (which covers OpenAI, OpenRouter, Groq, Together, DeepSeek, xAI, Ollama and any compatible gateway). Streaming, translation, retries. |
| `inertia-tools` | The tool registry pipeline (normalise, validate against the schema, ask permission, execute, truncate) and the built-in tools: files, search, shell and background processes, patch, worktrees, task list, plans, `present`, the browser pane tools, terminal reading, and the language-server tool. |
| `inertia-store` | The workspace folder: layout, atomic writes, generic record CRUD, settings documents, secrets, conversations, `SKILL.md` frontmatter. |
| `inertia-memory` | Memory records, scope rules, BM25 recall, the block injected into the prompt under a byte budget, the background capture pass, the three memory tools, and an MCP memory server as an alternative store. |
| `inertia-hooks` | Reads `hooks.json` files, matches handlers, runs them, merges their verdicts. |
| `inertia-mcp` | MCP client: JSON-RPC over stdio or streamable HTTP, handshake, a server's tools exposed as a `ToolProvider`. Ships a `fake-mcp-server` binary for tests. |
| `inertia-openapi` | Parses OpenAPI documents (JSON or YAML), resolves `$ref`s, turns operations into tools, attaches auth. |
| `inertia-composio` | Composio API client: catalogue, connections, actions as tools. |
| `inertia-voice` | ElevenLabs client: quota, voices, models, text to speech, transcription. |
| `inertia-computers` | The machine interface and its three providers (Docker, Daytona, local), the sandbox image definition, and the desktop half: screenshots, pointer and keyboard actions, opening and launching things. |
| `inertia-terminal` | A real pty per terminal tab, a screen emulator for what the shell paints, and working-directory reports. |
| `inertia-cookies` | Reads cookies from a Chromium- or Firefox-family browser profile on this machine and writes them into a browser Inertia owns. |
| `inertia-lsp` | Language server client: discovery, JSON-RPC framing, diagnostics after an edit, definition/references/hover/symbols. |
| `inertia-mock` | Fake implementations of the core traits: a scripted provider, recording tools, a scripted permission gate. A dev-dependency only, so the shipping binary cannot reach a mock. |
| `inertia-devkit` | A headless binary that runs the real agent loop against the mocks. See [development.md](development.md#the-headless-devkit). |

The dependency graph is shallow and acyclic:

```
                     inertia-core
                    (types + traits)
                           ▲
      ┌──────────┬─────────┼─────────┬──────────┬──────────┐
   store     provider    tools      mcp      openapi   composio   ...
      ▲          ▲         ▲         ▲          ▲         ▲
      └──────────┴─────────┴────┬────┴──────────┴─────────┘
                                │
                          inertia-agent ──► depends only on core
                                ▲
                  ┌─────────────┴─────────────┐
            inertia (app)               inertia-devkit
         real impls + Tauri            mock impls, headless
```

### Trait seams

Each seam in `inertia-core` has a real implementation and a fake one.

| Trait | Real | Fake | Abstracts |
|---|---|---|---|
| `Provider` | `inertia-provider` | scripted responses | a streaming model call |
| `Tool` | `inertia-tools`, app tools | recording fakes | one callable tool |
| `ToolProvider` | MCP, OpenAPI, Composio | in-memory catalogue | a source of tools that changes at runtime |
| `PermissionGate` | app (asks the window) | always-allow / scripted | approval decisions |
| `Lifecycle` | app (runs hooks) | none | named moments in a turn |
| `Clock` | system | fixed / advanceable | time, for deterministic tests |

`ToolProvider` being separate from `Tool` is what lets built-in, MCP, OpenAPI
and Composio tools compose into one flat registry. The agent loop cannot tell
them apart, and it cannot skip the permission check, because it only ever holds
the registry and never a tool directly.

## The app crate (`src-tauri/src`)

Thin by intent: it owns the window, the command surface, the wiring that picks
concrete implementations, and the tools that need the running app.

| Module | Role |
|---|---|
| `lib.rs` | Plugin setup, the `generate_handler![...]` command list, background tasks started at launch, tray icon, shutdown. |
| `state.rs` | `AppState`: the open workspace and the concrete providers, registries, stores and caches it holds. |
| `ws.rs` | The workspace command surface (`ws_*`): choosing/adopting a folder, records, documents, secrets, files. |
| `turn.rs` | `agent_run` and friends: builds a turn, translates agent events into the window's event vocabulary, permission replies. |
| `permission.rs` | The permission gate that raises an approval card and suspends the tool call until answered. |
| `tool_access.rs` | Optional on-demand tool loading (`load_tools`). |
| `inertia_tools.rs` | The `inertia_*` tools an agent uses to set Inertia itself up. |
| `computer_tools.rs`, `computers.rs` | Tools and commands over the computer providers. |
| `task_tool.rs`, `crew.rs`, `crew_commands.rs` | Sub-agents: blocking `task` and the background crew (`spawn`, `collect`, ...). |
| `group.rs`, `group_tools.rs` | Group conversations: several agents in one thread, `invite`/`handover`/`part`. |
| `routines.rs` | The routine scheduler and the `later` tool. |
| `skills.rs`, `memory.rs`, `hooks.rs`, `question.rs`, `failures.rs` | Skills, memory, lifecycle hooks, the `question` tool, the failure log. |
| `records.rs`, `changes.rs` | Turn records under `history/sessions`, and per-turn file snapshots with diff and undo. |
| `supervisor.rs` | Starts enabled MCP servers and reconnects them with backoff. |
| `preview.rs`, `terminal.rs` | The browser pane (child webviews) and terminal tabs. |
| `llm.rs`, `voice.rs`, `integrations.rs`, `commands.rs`, `project.rs`, `notify.rs`, `cookies.rs`, `desktop.rs` | Providers and one-shot completions, voice, MCP/OpenAPI/Composio commands, `AGENTS.md` setup, notifications/tray/launch at login, cookie import, small desktop errands. |
| `platform/window.rs` | Native window chrome on Windows (rounded corners via DWM). |

Background tasks started in `setup`: the MCP supervisor, the routine
scheduler, the crew panel feed, and terminal output streaming. On quit the app
stops terminal processes, shuts down language servers, closes open turn
records as interrupted, and drains pending memory-capture passes.

## The bridge (`src/bridge`)

The window never calls `invoke` directly. `src/bridge/` installs a set of
`window.<name>API` namespaces (`workspaceAPI`, `agentAPI`, `computerAPI`, ...)
backed by Tauri commands, and every screen goes through those. Some namespace
names (`electronAPI` for window and desktop errands) are historical.

- `envelope.js` turns every command result into `{ ok: true, data }` or
  `{ ok: false, error }`, and every Tauri event into a subscription that
  returns a synchronous unsubscribe.
- `contract.js` lists each namespace, its methods, and whether it is `live`,
  `partial` or `absent`. `bridge.test.js` fails if a declared method is not
  installed.

The two authoritative lists of what crosses the boundary are the
`generate_handler![...]` block in `src-tauri/src/lib.rs` and
`src/bridge/contract.js`. A command in the first that no bridge module calls is
dead and should be removed.

## State

There is no database. All durable state is files in the workspace folder (see
[workspace.md](workspace.md)), written atomically and read back on demand.
Records are plain JSON (`serde_json::Value`) end to end rather than typed Rust
structs, so a field one side does not know about survives a round trip.

Outside the workspace the app keeps exactly one file: a pointer in the
platform's app-config directory (`workspace.json`) that says where the
workspace folder is. Deleting it sends the app back to first-run setup without
touching the folder.

In-memory only: running turns, pending permission and question cards, the
crew run table, terminal ptys, MCP connections, language servers. None of these
survive a restart; turn records left "running" are settled as interrupted on
the next launch.

## Events

Commands return values; anything that streams or happens later arrives on a
Tauri event channel. Every record or document write emits
`workspace:changed`, so screens update when an agent, a routine or another
window changes something.

| Channel | Carries |
|---|---|
| `agent:event` | a turn's stream, plus permission and question asks (tagged by `channel`) |
| `workspace:changed` | a record or document was written |
| `computer:event` | a machine's state and screen |
| `terminal:event` | terminal output, cwd changes, exit |
| `preview:event` | browser pane navigation, console, network |
| `crew:event` | background crew runs |
| `routine:event` | a routine started, finished or was rescheduled |
| `composio:event` | a Composio connection or the catalogue changed |
| `llm:event` | a one-shot completion streaming (titles, `/compact`) |
| `notify:event` | something finished while the window was elsewhere |
| `hooks:run` | a lifecycle hook ran |

## How a turn runs

1. The window calls `agent_run` with the thread id, the transcript (already
   windowed and with attachments resolved), the agent, model, conversation
   mode and working folder. It returns a turn id at once.
2. The app resolves the agent record and model, assembles the system prompt
   (the agent's identity and instructions, the core prompt, the mode
   paragraph, the environment: folder, platform, shell, date; `AGENTS.md` and
   `.inertia/rules/` from the project, advertised skills, injected memories),
   and builds a tool registry for this turn: built-ins, app-level tools, MCP,
   OpenAPI and Composio tools, minus anything the mode withholds or a rule
   denies unconditionally.
3. Before the first write, a snapshot of the working folder is taken so the
   turn's changes can be diffed and undone.
4. `inertia-agent` loops: stream a model response; if it asked for tools, run
   each through the registry (normalise, validate, `PreToolUse` hooks, ask
   permission, execute, truncate, `PostToolUse` hooks); append results; repeat.
   It stops when the model answers without tool calls, the step budget runs
   out, the turn is cancelled, or the provider fails. `Stop` hooks can send it
   back for another step.
5. Every event is translated to the window's vocabulary (`delta`,
   `tool-start`, `tool-end`, `usage`, `done`, ...) and emitted on
   `agent:event`, and also appended to a turn record under `history/sessions`
   so a reloaded window can rejoin a running turn.
6. When the conversation goes quiet, a memory-capture pass may extract durable
   facts in the background.

Text typed while a turn is running is delivered with `agent_steer` and reaches
the model at the next step.

## Permissions and approvals

Every tool call names a permission **key** (`read`, `edit`, `shell`,
`browser`, `computer`, `mcp`, ...) and a **target** (the path, the command, the
URL). Rules are `{ tool, pattern, action }` with `action` one of `allow`,
`ask`, `deny`; `tool` and `pattern` are globs supporting only `*` and `?`. The
most specific matching rule wins; nothing matching means `ask`.

Rules live in `settings/permissions.json`, layered workspace then per-agent.
An `ask` raises a card in the window and suspends the tool call; no answer
(card dismissed, window closed, turn cancelled) is a refusal. "Always allow"
writes a rule. Each conversation also has a mode (Chat, Plan, Autonomous,
Group) that decides which tools the turn holds at all.

The full model is in [tools.md](tools.md#permissions) and
[security.md](security.md).

## Computers

A computer is a record in `computers/` naming a provider and that provider's
handle for the machine. `inertia-computers` defines one interface (status,
exec, files, snapshots, screen) and three implementations:

- **Docker**: a container built from the sandbox image in
  `src-tauri/crates/inertia-computers/sandbox/`, which is compiled into the
  binary and built locally on first use.
- **Daytona**: a cloud sandbox created from a snapshot of the same image.
- **Local**: a folder under the workspace and the host's own shell. Not a
  sandbox.

Desktop actions are compiled to `xdotool` and screenshots taken with `import`
inside the machine, so the same code drives a Docker container and a Daytona
sandbox. The live view is noVNC served from inside the machine. See
[computers.md](computers.md).

## Conventions

- **Errors**: `thiserror` enums in library crates; `anyhow` only in the app and
  devkit binaries. Command errors are `String`s written to be shown to a
  person as-is.
- **Tool schemas**: JSON Schema per tool, validated before the tool runs.
  Validation failures go back to the model as a failed call, not a turn error.
- **Secrets in memory**: API keys are wrapped in `secrecy::SecretString` so a
  stray `{:?}` cannot print one.
- **Time**: `jiff`, through the `Clock` seam where tests care.
- **Async**: Tokio. Blocking filesystem and process work goes through
  `spawn_blocking`.
- **Lints**: workspace lints in `src-tauri/Cargo.toml` (`unwrap_used`,
  `expect_used`, `await_holding_lock`, ...) are warnings locally and errors in
  CI (`clippy -D warnings`).

## Testing

Every crate has unit tests next to its code, and some have integration tests in
`tests/`. The window has Vitest tests (`*.test.js`) for `shared/`, the bridge
contract and the chat logic. CI (`.github/workflows/ci.yml`) runs the frontend
tests and build, and Rust clippy and tests on `windows-latest`. See
[development.md](development.md#tests-and-checks).
