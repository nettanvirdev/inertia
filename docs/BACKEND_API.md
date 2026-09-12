# Backend API

Every Tauri command the frontend can call, and every event it can listen for.
This is the whole surface — nothing else crosses the boundary.

Call commands with `invoke` from `@tauri-apps/api/core`, listen with `listen`
from `@tauri-apps/api/event`. Wrap each domain in one module under
`src/lib/api/`, following the pattern in `src/lib/prefs.ts`, so a rename on the
Rust side touches one file rather than forty components.

**Errors arrive as rejected promises**, not as an `{ok, data}` envelope. The
message is written to be shown to a person as-is.

```ts
try {
  await invoke("thread_save", { thread });
} catch (e) {
  toast(String(e)); // e.g. "No workspace is open."
}
```

## Workspace

A workspace must be open before anything else works. Every other command
rejects with `"No workspace is open."` until one is.

| Command | Arguments | Returns |
|---|---|---|
| `workspace_open` | `{ root: string }` | `string` — the opened path |
| `workspace_current` | — | `string \| null` |

Opening creates the folder structure if it isn't there, so pointing at an empty
directory is a valid way to start a new workspace.

## Conversations

| Command | Arguments | Returns |
|---|---|---|
| `threads_list` | — | `Thread[]`, newest first |
| `thread_save` | `{ thread: Thread }` | — |
| `thread_delete` | `{ id: string }` | — |
| `messages_read` | `{ threadId: string, live?: string[] }` | `Message[]` |
| `messages_save` | `{ threadId: string, messages: Message[] }` | — |

**`live` matters.** Pass the ids of messages this window is still streaming.
Anything else found mid-stream is assumed abandoned by a crash and is settled —
marked stopped, with unfinished tool calls failed — so a conversation never
reopens stuck showing a Stop button with no turn behind it. When loading cold,
pass nothing.

```ts
type Thread = {
  id: string;
  agentId?: string;
  title: string;
  mode: "chat" | "plan" | "autonomous";
  approval: string;          // "ask" | "edits" | "auto"
  pinned: boolean;
  unread: number;
  updatedAt: string;         // RFC 3339
  createdAt?: string;
  preview: string;
  messageCount: number;
  computerAttached?: string;
};

type Message = {
  id: string;
  role: "user" | "agent";
  agentId?: string;
  content: string;
  createdAt: string;
  status?: "sent" | "streaming" | "error";
  parts?: Part[];
  stopped?: boolean;
  error?: string;
  model?: string;
};

type Part =
  | { type: "text"; text: string }
  | {
      type: "tool";
      callId?: string;
      name: string;
      arguments: string;     // raw JSON string as the model produced it
      state: "running" | "done" | "failed";
      output?: string;
      ok?: boolean;
    };
```

## Settings

| Command | Arguments | Returns |
|---|---|---|
| `models_get` | — | `Models` |
| `models_save` | `{ models: Models }` | — |
| `models_probe` | `{ providerId: string }` | `ModelInfo[]` |
| `permissions_get` | — | `Permissions` |
| `permissions_save` | `{ permissions: Permissions }` | — |

```ts
type Models = {
  providers: Provider[];
  defaultModel: string;      // "anthropic/claude-sonnet-4-5"
};

type Provider = {
  id: string;
  name: string;
  enabled: boolean;
  baseUrl: string;
  apiKeySecret?: string;     // the NAME of a secret, never the key
  kind?: "anthropic" | "openai";  // inferred from baseUrl when absent
  headers?: Record<string, string>;
  models?: { id: string; label: string; context?: number }[];
};

type Permissions = {
  workspace: Rule[];
  agents: Record<string, Rule[]>;
};

type Rule = {
  tool: string;              // a glob: "shell", "mcp_*"
  pattern: string;           // a glob over the argument: "git push *", "*"
  action: "allow" | "ask" | "deny";
};
```

**Never put an API key in `models.json`.** `apiKeySecret` holds the *name* of
an entry in the workspace's secret store; the value lives elsewhere. This is
what lets a workspace folder be committed to git or attached to a bug report.

`models_probe` asks the provider what models it offers. An endpoint with no
model list returns `[]` rather than failing — plenty of gateways don't
implement the route, and the user can still type a model id.

## Running a turn

```ts
const { turnId } = await invoke<{ turnId: string }>("agent_send", {
  threadId,
  text,
  model: "anthropic/claude-sonnet-4-5",  // optional; defaults to defaultModel
  agentId: undefined,                     // optional
});
```

`agent_send` **returns immediately** with a turn id. The turn runs in the
background and reports through events. Do not await the reply — awaiting would
block the bridge for the whole turn and there would be nothing to stream.

`agent_stop({ turnId })` → `boolean`. Cancels the turn, drops the in-flight
provider request, and refuses any approval card still on screen. Returns false
if that turn already finished.

The backend persists the conversation when the turn ends, so a window closed
mid-turn still leaves the messages on disk.

### Listening

```ts
import { listen } from "@tauri-apps/api/event";

const stop = await listen("agent:event", ({ payload }) => {
  if (payload.turnId !== turnId) return;   // several turns can run at once
  switch (payload.type) { /* ... */ }
});
```

Every event carries `turnId` and `threadId` alongside its own fields.

| `type` | Fields | Meaning |
|---|---|---|
| `start` | `model` | the turn began |
| `step` | `step` | one provider call plus its tools; 1-based |
| `delta` | `text` | prose, as it arrives — append it |
| `reasoning` | `text` | visible reasoning |
| `toolStarted` | `call`, `title?` | a call is about to run |
| `toolFinished` | `result` | it finished; match by `result.callId` |
| `notice` | `message` | something was worked around; not a failure |
| `done` | `stopped`, `history`, `usage?` | terminal |

Pair `toolStarted` with `toolFinished` **by call id, not by order** — calls in
one batch finish in whatever order they finish. Every finished call is
guaranteed to have been announced first.

`stopped` is `{ reason: "complete" | "maxSteps" | "error" | "cancelled" |
"refused", message? }`. Only `error` is a failure; `cancelled` is the user
pressing Stop and should not be shown as one.

## Approvals

When a tool needs permission the backend emits `permission:ask` and the call
blocks until answered.

```ts
type Ask = {
  id: string;
  key: string;       // "shell", "edit", "read", "doom_loop"
  target: string;    // the exact thing — the command, the file path
  always?: string;   // present only when "always allow" is meaningful
};

await invoke("permission_respond", { id, answer: "allow" });
// "allow" | "allowAlways" | "deny"
```

Three things to get right:

- **Show `target` verbatim.** It is the exact command or path, and it is what
  a permission rule would be written against. Paraphrasing it in the UI means
  the user approves something different from what runs.
- **Offer "always allow" only when `always` is present.** When absent there is
  nothing sensible to remember, and `allowAlways` behaves as a plain allow.
  Choosing it writes a rule into the workspace permissions.
- **No answer is a refusal.** If the card is dismissed or the window closes,
  the call is denied. Never default to allow.

A `doom_loop` ask means the model has made the same call with the same
arguments several times running. It is asking whether to keep going, and the
honest default is no.

## What isn't here yet

MCP servers, OpenAPI imports, Composio connections, routines, memory, agents
and skills are specified and the storage layout reserves their folders, but
they have no commands yet. They will follow this same shape: one command group,
one event channel, camelCase fields.
