# Backend architecture

How the Rust side of Inertia is organised, and why.

## The frontend is a copy, not a rewrite

`src/` is the Electron app's renderer (`D:\oss\inertia/src/renderer`), copied
verbatim — 278 files, ~43k lines, its own `globals.css`, its own icon set. It is
not a reimplementation and should not become one. Pixel parity is achievable
here only because the pixels are drawn by the same code.

The renderer talks to seventeen `window.<something>API` namespaces that
Electron's `src/main/preload.cjs` installs. `src/bridge/` installs the same
namespaces backed by Tauri commands, so no screen knows which shell it is in.
One line of `src/main.jsx` — `import "@/bridge"` — is the only deviation from
the original, and it is first because two modules read their bridge while they
evaluate.

**Namespaces with no backend yet are left undefined on purpose.** Every consumer
in the renderer reads its bridge through a guarded accessor and has a path for
"not available here". A pane that says a feature is unavailable is the truth; a
stub answering `[]` is a screen that looks like it works.

`src/bridge/contract.js` is the machine-readable version of that preload, and
`bridge.test.js` holds the shell to it. It exists because every bridge bug so
far reached the user as a screenshot — a pane reading "needs the desktop app"
because a namespace was never installed, or a raw `is not a function` because
one method was missed inside a namespace that was. Both are trivially
checkable; neither was being checked.

| Namespace | State |
|---|---|
| `electronAPI` | live — window chrome, app info, snippets, images, clipboard |
| `workspaceAPI` | live — the generic record store, documents, secrets, files |
| `mcpAPI` | live — connect, disconnect, test, per-server tools |
| `openapiAPI` | live — import, operations, details, and calling one operation to prove a base URL and a key |
| `composioAPI` | live — catalogue, connect, permissions, logos |
| `projectAPI` | live — `AGENTS.md` and `.inertia/rules/` |
| `agentAPI` | live — turns, tools, permissions, grants, group conversations (the room, the floor, `invite`/`handover`/`part`), turn records and a thread's history, summarising for `/compact`, steering a running turn, and the questions a tool asks mid-call |
| `llmAPI` | live — providers, models, test, and the one-shot streaming `chat` that titles a thread and compacts a conversation |
| `voiceAPI` | live — ElevenLabs: quota, voices, models, speech, transcription |
| `computerAPI` | live — Docker, Daytona and local machines, the sandbox image built from a Dockerfile compiled into the binary, the live screen and driving it. Importing a browser profile's cookies is the one part left |
| `terminalAPI` | live — a real pty per tab, the shell the rest of the app uses, working-directory reports, and `terminal_read` so the agent can see what the person ran |
| `previewAPI` | live — a child webview layered over the window, with navigation, history, console and network logs, and nine tools to drive it. A Tauri webview cannot photograph a page, so there is no screenshot |
| `crewAPI` | live — the background-crew panel over a real table of runs (`spawn`/`collect`/`team`/`wait`/`followup`/`interrupt`/`agent_send`). Pausing and restarting a run answer honestly that the machinery is not there |
| `hooksAPI` | live — the file, its warnings, recent runs; `PreToolUse`, `PostToolUse` and `Stop` fire in the turn, a `prompt` handler asks the turn's own model with no tools, and every run appears in the transcript |
| `memoryAPI` | live — search, which project is open, capture on demand. The Memory *screen* reads the `memory` collection over the workspace bridge; the backend routes that one collection across both stores |
| `snapshotAPI` | live — a content-addressed snapshot taken before a turn's first write, the diff after it, and Undo |
| `routineAPI` | live — a scheduler that keeps time without a window, in the routine's own mode and approval |
| `notifyAPI`, `backgroundAPI` | live — a banner when a turn finishes out of sight, a tray icon, minimise-to-tray and launch-at-login |

## Where it stands

| Crate | What works | Tests |
|---|---|---|
| `inertia-core` | domain types, errors, permission engine, every trait seam | 61 |
| `inertia-tools` | registry pipeline, schema validator, match ladder, truncation, and the tools themselves: reading, writing, editing, patching, searching, listing, a PowerShell-backed shell with background commands, worktrees, task lists, plans, presenting, and asking a language server | 239 |
| `inertia-provider` | Anthropic Messages and OpenAI chat-completions, streaming, translation | 106 |
| `inertia-store` | atomic writes, workspace layout, generic record CRUD, frontmatter and SKILL.md, secrets, conversations, settings | 115 |
| `inertia-agent` | the turn loop, step budget, repetition guard, prompt assembly, the four conversation modes and what each withholds, the group floor, one transcript seen from one seat, and steering a turn that is already running | 121 |
| `inertia-hooks` | the hook file, the matcher, running a handler, merging verdicts | 51 |
| `inertia-memory` | scope and duplicate rules, BM25 recall, the injected block under a byte budget, the capture pass, the three memory tools, and an MCP memory server as an alternative store | 82 |
| `inertia-mcp` | JSON-RPC, stdio and streamable-HTTP transports, handshake, tools as a provider | 48 |
| `inertia-openapi` | JSON/YAML specs, `$ref` resolution, operations as tools, auth | 53 |
| `inertia-composio` | API client, paging, connections, actions as tools | 36 |
| `inertia-voice` | ElevenLabs client: quota, voices, models, speech, transcription | 12 |
| `inertia-computers` | the machine seam with Docker, Daytona and local providers, the sandbox image, and the desktop half: observing a screen, driving it, opening and launching | 84 |
| `inertia-mock` | scripted provider, recording gate and tools | 15 |
| `inertia-lsp` | JSON-RPC framing, server discovery, diagnostics after an edit, and the questions `grep` cannot answer | 46 |
| `inertia-terminal` | a pty per tab, a grid emulator for what ConPTY paints, and working-directory reports | 29 |
| app (`src/`) | window chrome, command surface, UI permission gate, delta coalescing, the group room and its three tools, `task` and the crew, the seven `inertia_*` setup tools, skills, routines, turn records, failures, questions, snapshots, the tray, and on-demand tool loading | 363 |

One rule about the app crate's own tests, because it costs an hour to find:
**never build an `AppState` inside a `#[cfg(test)]` there.** Doing so links
Tauri's window chrome into the test harness, and on Windows the whole binary
then fails to start with `STATUS_ENTRYPOINT_NOT_FOUND` before a single test
runs - so every test in the crate fails, none of them for its own reason. Build
what the test needs directly (`Workspace::open`, `Rooms::default`), and take an
emitter or a small trait where a type would otherwise want an `AppHandle`.

1,461 tests, no clippy warnings. `cargo run -p inertia-devkit` runs the whole
agent loop headless against mocks, and

```
cargo run -p inertia-store --example scan -- D:\Inertia
```

reads a real workspace folder and reports what came back. A unit test proves the
code agrees with itself; that proves it agrees with a folder the Electron app
wrote over months of use, which is the only thing that matters for the port.

**Not built yet**: context compaction, routines, the background crew
(`spawn`/`collect`/`team`/`wait` — one-shot `task` delegation is built), the
sandbox. The storage layout reserves their folders and the port
specs in `docs/port/` describe them.

**One caveat on the release profile**: `lto = "true"` (fat) has crashed `rustc`
once here under memory pressure, and recovered on retry. If it recurs,
`lto = "thin"` costs very little and is far more robust.

## The one rule

**Dependencies point inward, toward `inertia-core`, and nothing in `crates/`
knows Tauri exists.**

Everything else in this document follows from that. If a `crates/` member
reaches for `tauri::`, the dependency is pointing the wrong way and the fix is
to move the offending call up into the app crate, not to add the import.

## Crate topology

```
src-tauri/
  Cargo.toml            workspace root, and the Tauri app package
  src/                  the shell: window chrome, commands, wiring
    platform/window.rs  Win32 chrome (rounded corners, work area, animation)
    prefs.rs            pre-workspace preferences
  crates/
    inertia-core        domain types + the traits everything else implements
    inertia-store       the on-disk workspace: sessions, settings, records
    inertia-provider    Provider impls (Anthropic, OpenAI, ...)
    inertia-tools       builtin Tool impls (shell, edit, search, ...)
    inertia-hooks       the user's own programs, run at moments of a turn
    inertia-memory      what an agent knows before the conversation starts
    inertia-mcp         MCP servers as a ToolProvider
    inertia-openapi     an OpenAPI spec as a ToolProvider
    inertia-composio    Composio actions as a ToolProvider
    inertia-voice       ElevenLabs: speech in, speech out
    inertia-computers   machines an agent can drive (docker, daytona, local)
    inertia-agent       the turn loop; depends on traits, never on impls
    inertia-mock        fake impls of every trait
    inertia-devkit      headless binary: the agent wired to mocks
```

The dependency graph is acyclic and shallow:

```
                     inertia-core
                    (types + traits)
                           ▲
      ┌──────────┬─────────┼─────────┬──────────┬──────────┐
   store     provider    tools      mcp      openapi   composio
      ▲          ▲         ▲         ▲          ▲         ▲
      └──────────┴─────────┴────┬────┴──────────┴─────────┘
                                │        (impls; siblings, never peers)
                          inertia-agent ──► depends only on core
                                ▲
                  ┌─────────────┴─────────────┐
            inertia (app)               inertia-devkit
         real impls + Tauri            mock impls, headless
```

`inertia-agent` is the load-bearing piece. It contains the turn loop, prompt
assembly, and tool dispatch, and it is written entirely against traits from
`inertia-core`. It cannot name Anthropic, cannot name the filesystem, and
cannot name MCP. That is what makes the same loop runnable against mocks.

## Why a workspace and not one crate with modules

Module boundaries inside a crate are advisory: nothing stops `agent::loop` from
calling `provider::anthropic::sign_request` two years from now, and by the time
anyone notices, the seam is gone. Crate boundaries are enforced by the
compiler. Given that the explicit goal is a backend that stays rewireable, the
boundary needs teeth.

The costs are real but small here: slightly more manifest bookkeeping, and
incremental rebuilds that are in practice *faster*, because touching a tool
implementation no longer forces the agent loop to recompile.

## The trait seams

Each of these lives in `inertia-core` and has at least two implementations —
the real one and a mock. That is the test for whether a seam is honest.

| Trait | Real | Mock | What it abstracts |
|---|---|---|---|
| `Provider` | `inertia-provider` | scripted responses | a streaming LLM call |
| `Tool` | `inertia-tools` | recording fakes | one callable tool |
| `ToolProvider` | mcp / openapi / composio | in-memory catalog | a *source* of tools |
| `Store` | `inertia-store` | in-memory tree | workspace persistence |
| `PermissionGate` | app (asks the UI) | always-allow / scripted | approval decisions |
| `Lifecycle` | app (`TurnHooks`) | none attached | the named moments of a turn |
| `Complete` | app (`ModelPass`) | a fixed reply | the one model call memory makes on its own |
| `Clock` | system | fixed/advanceable | time, for deterministic tests |

`ToolProvider` being separate from `Tool` is what lets MCP, OpenAPI, Composio
and the builtins compose into one flat registry that the model sees as a single
tool list, without the agent loop knowing which source any given tool came
from.

## Mocks are a crate, not a folder

The obvious alternative was a separate project for mock providers. In-workspace
wins for one reason: integration tests can depend on `inertia-mock` directly,
so the fakes are exercised by CI and cannot rot. A sibling folder outside the
workspace would drift out of sync the first week.

`inertia-devkit` is the payoff — a headless binary that wires the real agent
loop to a mock provider, mock MCP servers, mock Composio, and a temp-dir store.
It runs a full conversation with tool calls and permission prompts without a
network, an API key, or a window. That is the loop to develop against; the
Tauri app is then just a different set of arguments to the same constructor.

## Conventions

- **Errors**: `thiserror` enums per crate, `anyhow` only in the app and devkit
  binaries. A library returning `anyhow` forces callers to match on strings.
- **Tool schemas**: derived with `schemars` from the same struct that parses
  the input, so a schema cannot drift from its parser.
- **Secrets**: API keys are `secrecy::SecretString`, so a stray `{:?}` cannot
  print one into a log file.
- **Time**: `jiff`, and always through the `Clock` seam in code that tests care
  about.
- **Async**: Tokio throughout. Blocking filesystem and process work goes
  through `spawn_blocking` rather than blocking a runtime worker.
