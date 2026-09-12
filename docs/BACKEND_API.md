# Backend API

How the frontend and the Rust side talk, and where the authoritative list of
what they can say lives.

This file is a map, not a catalogue. A hand-written table of two hundred
commands is a table that is wrong within a week, and the last one was: it
documented a conversation API the window had stopped calling and closed with a
list of features that "have no commands yet" — every one of which had shipped.

## The two authoritative lists

- **`src-tauri/src/lib.rs`**, the `generate_handler![...]` block. Every command
  that exists, grouped and commented. Nothing crosses the boundary that is not
  in it.
- **`src/bridge/contract.js`**, the namespaces the window is written against,
  with each one's methods and whether it is `live`, `partial` or `absent`.
  `bridge.test.js` holds the shell to it, so a method that is declared and not
  installed fails a test rather than a click.

Between them sits `src/bridge/`, one module per namespace. A rename on the Rust
side touches one file there rather than forty components.

## The envelope

The window was written against Electron's preload, where every call answers
`{ ok: true, data }` or `{ ok: false, error }` and every subscription is a
synchronous function returning a synchronous unsubscribe. Tauri offers neither.
`src/bridge/envelope.js` is the entire difference between the two.

```js
import { call, raw, subscribe } from "./envelope";

const answer = await call("ws_list", { collection: "agents" });
if (!answer.ok) toast(answer.error);        // a sentence, written for a person
```

`raw` is the same call without the envelope, for the handful of places the
window reads a plain value (`info?.version`, `result?.saved`). `subscribe`
turns a Tauri channel into the synchronous unsubscribe the renderer expects.

Errors from a command are `String` on the Rust side, written to be shown as-is.

## The workspace surface

Most screens never name a command. They go through `ws_*`, which speaks in
collections and documents rather than paths:

| Command | For |
|---|---|
| `ws_list` / `ws_get` / `ws_put` / `ws_patch` / `ws_remove` / `ws_rename` | records in a collection — `agents`, `routines`, `memory`, `threads` |
| `ws_doc_get` / `ws_doc_set` | a single named document — `models`, `permissions`, `identity` |
| `ws_secret_*` | the secret store, which holds values `models.json` only names |
| `ws_file_*` | bytes in the workspace folder |
| `ws_tree` / `ws_browse` / `ws_list_dir` / `ws_inspect` | the folder, as a folder |

Records are JSON all the way down. There is no typed Rust mirror of a record,
on purpose: a mirror is a second definition of the shape, and the half that is
behind quarantines the user's data as unreadable.

**`workspace:changed`** is emitted whenever a record or document is written —
by a screen, by an agent mid-turn, or by a routine. Every screen listens; it is
what makes a memory an agent just saved appear without a relaunch.

## Running a turn

```js
const { data } = await agent.run({ threadId, text, model, agentId, mode, approval });
// data.id — the turn id. Returns immediately.
```

The turn runs in the background and reports on **`agent:event`**. Awaiting it
would block the bridge for the whole turn, and there would be nothing to
stream. `agent_cancel`, `agent_steer` and `agent_active` address a turn by that
id; `agent_active` is what lets a reopened window tell a quiet turn from a dead
one.

Every event carries the turn and thread it belongs to.

| `type` | Fields | Meaning |
|---|---|---|
| `step` | `step` | one provider call plus its tools; 1-based |
| `delta` | `text` | prose, as it arrives — append it |
| `reasoning` | `text` | visible reasoning |
| `steer` | `text` | something typed mid-turn, echoed where it reached the model |
| `tool-start` | `callId`, `name`, `title`, `args` | a call is about to run |
| `tool-end` | `callId`, `ok`, `output`, `metadata`, `durationMs` | it finished |
| `warning` | `kind`, `message` | worked around; not a failure |
| `usage` | `usage`, `context` | token counts, and how full the window is |
| `error` | `message` | the turn failed; a `done` follows |
| `done` | `stopped`, `usage` | terminal |

Pair `tool-start` with `tool-end` **by call id, not by order** — calls in one
batch finish in whatever order they finish.

`stopped` is `complete`, `maxSteps`, `error`, `cancelled` or `refused`. Only
`error` is a failure; `cancelled` is the person pressing Stop.

`context` on the `usage` event is what draws the gauge in the chat header and
what decides when a conversation has to be summarised. A turn that does not
send it is a conversation that runs until the provider refuses it.

## The two things that suspend a tool

A tool can stop mid-call and wait for a person: an approval, or a question the
model asked. Both arrive on `agent:event` tagged with a `channel` rather than
on channels of their own — one subscription, so a window cannot end up
listening for answers and missing questions.

```js
// channel: "permission", type: "asked"
{ channel: "permission", type: "asked", id, question: { key, target, always? } }
await agent.reply(id, "allow");        // "allow" | "allowAlways" | "reject"
```

- **Show `target` verbatim.** It is the exact command or path, and it is what a
  rule would be written against. Paraphrasing it means approving something
  other than what runs.
- **Offer "always" only when `always` is present.** Choosing it writes a rule
  into the workspace permissions.
- **No answer is a refusal.** A dismissed card or a closed window denies.

`permission_waiting` and `agent_questions_waiting` replay what is still open,
so a reopened window draws the cards it walked away from.

## Every event channel

| Channel | Bridge | Carries |
|---|---|---|
| `agent:event` | `agentAPI` | a turn, plus permission and question asks |
| `workspace:changed` | `workspaceAPI` | a record or document was written |
| `computer:event` | `computerAPI` | a machine's state, and its screen |
| `terminal:event` | `terminalAPI` | `data`, `cwd`, `exit`, `reveal` |
| `preview:event` | `previewAPI` | the browser pane: navigation, console, network |
| `crew:event` | `crewAPI` | what the team is doing |
| `routine:event` | `routineAPI` | a routine started, finished, or was rescheduled |
| `composio:event` | `composioAPI` | a connection or the catalogue changed |
| `llm:event` | `llmAPI` | a bare completion, streaming |
| `notify:event` | `notifyAPI` | something finished while the person was elsewhere |
| `hooks:run` | `hooksAPI` | a lifecycle hook fired |
