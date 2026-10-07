# Backend API

How the window and the Rust side talk, and where the authoritative list of
what they can say lives.

This is a map, not a catalogue. A hand-written table of two hundred commands
goes stale within a week; the code does not.

## The two authoritative lists

- **`src-tauri/src/lib.rs`**, the `generate_handler![...]` block. Every command
  that exists, grouped and commented. Nothing crosses the boundary that is not
  in it.
- **`src/bridge/contract.js`**, the namespaces the window is written against,
  with each one's methods and whether it is `live`, `partial` or `absent`.
  `bridge.test.js` holds the bridge to it, so a method that is declared and not
  installed fails a test rather than a click.

Between them sits `src/bridge/`, one module per namespace. A rename on the Rust
side touches one file there rather than forty components.

## The envelope

The screens expect every call to answer `{ ok: true, data }` or
`{ ok: false, error }`, and every subscription to be a function returning a
synchronous unsubscribe. Tauri offers neither. `src/bridge/envelope.js` is the
whole difference.

```js
import { call, raw, subscribe } from "./envelope";

const answer = await call("ws_list", { collection: "agents" });
if (!answer.ok) toast(answer.error);        // a sentence, written for a person
```

`raw` is the same call without the envelope, for the places the window reads a
plain value (`info?.version`, `result?.saved`). `subscribe` turns a Tauri event
into the synchronous unsubscribe the screens expect.

Errors from a command are `String` on the Rust side, written to be shown as-is.

## The workspace surface

Most screens never name a specific command. They go through `ws_*`, which
speaks in collections and documents rather than paths:

| Command | For |
|---|---|
| `ws_list` / `ws_get` / `ws_put` / `ws_patch` / `ws_remove` / `ws_rename` | records in a collection: `agents`, `routines`, `memory`, `threads`, `skills`, ... |
| `ws_doc_get` / `ws_doc_set` | a single named document: `settings.models`, `settings.permissions`, `settings.identity`, ... |
| `ws_secret_list` / `ws_secret_get` / `ws_secret_set` / `ws_secret_remove` | the secret store, which holds the values `models.json` and plugin records only name |
| `ws_file_*` | bytes in the workspace folder |
| `ws_tree` / `ws_browse` / `ws_list_dir` / `ws_inspect` | the folder, as a folder |
| `ws_status` / `ws_configure` / `ws_reset` / `ws_wipe` / `ws_reveal` / `ws_choose_folder` | choosing, adopting, forgetting or emptying the workspace |

Collection and document keys are defined in
`src-tauri/crates/inertia-store/src/layout.rs`. Records are JSON all the way
down; there is no typed Rust mirror of a record, so a field one side does not
know about is preserved.

**`workspace:changed`** is emitted whenever a record or document is written,
by a screen, by an agent mid-turn, or by a routine. Every screen listens; it is
what makes a memory an agent just saved appear without a reload.

## Running a turn

```js
const { data } = await agentAPI.run({
  threadId, agentId, modelRef, history, messageId,
  conversationMode, cwd,
});
// data.id is the turn id. The call returns immediately.
```

`history` is the transcript as the window folded it, already windowed and with
attachments resolved; the new user message is its last entry. Group
conversations add `primaryAgentId`, `mentioned`, `roster` and `speaker`. The
full request shape is `RunRequest` in `src-tauri/src/turn.rs`.

The turn runs in the background and reports on **`agent:event`**.
`agent_cancel`, `agent_steer` and `agent_active` address a turn by its id;
`agent_active` is what lets a reopened window tell a quiet turn from a dead
one, and `agent_record` replays a turn's events from its record.

Every event carries the turn and thread it belongs to.

| `type` | Fields | Meaning |
|---|---|---|
| `step` | `step` | one provider call plus its tools; 1-based |
| `delta` | `text` | prose, as it arrives; append it |
| `reasoning` | `text` | visible reasoning |
| `steer` | `text` | something typed mid-turn, echoed where it reached the model |
| `tool-start` | `callId`, `name`, `title`, `args` | a call is about to run |
| `tool-end` | `callId`, `ok`, `output`, `metadata`, `durationMs` | it finished |
| `warning` | `kind`, `message` | worked around; not a failure |
| `usage` | `usage`, `context` | token counts, and how full the context window is |
| `error` | `message` | the turn failed; a `done` follows |
| `done` | `stopped`, `usage` | terminal |

Pair `tool-start` with `tool-end` **by call id, not by order**: calls in one
batch finish in whatever order they finish.

`stopped` is `complete`, `maxSteps`, `error`, `cancelled` or `refused`. Only
`error` is a failure; `cancelled` is the person pressing Stop.

## The two things that suspend a tool

A tool can stop mid-call and wait for a person: an approval, or a question the
model asked with the `question` tool. Both arrive on `agent:event` tagged with
a `channel`, so a window cannot end up listening for one and missing the other.

```js
// channel: "permission", type: "asked"
{ channel: "permission", type: "asked", id, question: { id, sessionId, key, target, always? } }
await agentAPI.reply(id, "allow");     // "allow" | "always" | anything else refuses
```

- **Show `target` verbatim.** It is the exact command or path, and it is what
  a rule would be written against.
- **Offer "always" only when `always` is present.** Choosing it writes an
  allow rule for `key` and the `always` pattern into
  `settings/permissions.json`.
- **No answer is a refusal.** A dismissed card, a cancelled turn or a closed
  window denies.

A `settled` event on the same channel removes a card, whichever way it ended.
`permission_waiting` and `agent_questions_waiting` replay what is still open,
so a reopened window can draw the cards it walked away from.

## Every event channel

| Channel | Bridge | Carries |
|---|---|---|
| `agent:event` | `agentAPI` | a turn, plus permission and question asks |
| `workspace:changed` | `workspaceAPI` | a record or document was written |
| `computer:event` | `computerAPI` | a machine's state, and its screen |
| `terminal:event` | `terminalAPI` | output, cwd, exit |
| `preview:event` | `previewAPI` | the browser pane: navigation, console, network |
| `crew:event` | `crewAPI` | what the background crew is doing |
| `routine:event` | `routineAPI` | a routine started, finished, or was rescheduled |
| `composio:event` | `composioAPI` | a connection or the catalogue changed |
| `llm:event` | `llmAPI` | a bare completion, streaming |
| `notify:event` | `notifyAPI` | something finished while the person was elsewhere |
| `hooks:run` | `hooksAPI` | a lifecycle hook ran |

## Adding a command

1. Write the `#[tauri::command]` in the module for its domain under
   `src-tauri/src/`, keeping logic in a crate where it can be tested without
   Tauri.
2. Register it in `generate_handler![...]` in `src-tauri/src/lib.rs`.
3. Expose it from the matching module in `src/bridge/`, and add the method to
   `src/bridge/contract.js`.
4. If it needs a new Tauri capability (a window or plugin permission), add it
   to `src-tauri/capabilities/default.json`.
