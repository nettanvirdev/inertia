# Integrations

Inertia can give agents tools from three kinds of outside source, all managed
on the **Integrations** screen and stored under `<workspace>/plugins/`. Their
tools join the built-in ones in a single list, and every call goes through the
same permission pipeline (see [tools.md](tools.md#permissions)). It can also
run your own programs at points in a turn, through hooks.

Credentials are never written into these records. A record names a secret, and
the value lives in `secrets/secrets.json` (**Settings > Secrets**).

## MCP servers

[Model Context Protocol](https://modelcontextprotocol.io) servers, over two
transports:

- **stdio**: Inertia starts the server as a child process and speaks
  newline-delimited JSON-RPC over its stdin/stdout.
- **http**: a URL that accepts one POST per message (streamable HTTP).

A server record, `plugins/mcp/<id>.json`:

```json
{
  "id": "github",
  "name": "github",
  "type": "stdio",
  "enabled": true,
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-github"],
  "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "{secret:GITHUB_PAT}" }
}
```

```json
{
  "id": "docs",
  "name": "docs",
  "type": "http",
  "enabled": true,
  "url": "https://mcp.example.com/mcp",
  "headers": { "Authorization": "Bearer {secret:DOCS_TOKEN}" },
  "timeoutMs": 60000
}
```

- **`{secret:NAME}` placeholders** in `env` values and `headers` values are
  replaced with the named secret when the server is connected. The record on
  disk keeps the placeholder, so it can be shared or screenshotted. An unknown
  name is left as written, so the failure is visible at the far end.
- Enabled servers are started when the app starts (or when a workspace is
  opened) and kept up: a server that exits is reconnected with backoff (5 s,
  10 s, 20 s, ... up to 5 minutes).
- A server's tools are named `<server>_<tool>` and ask under the `mcp` key with
  the tool name as the target, so a rule like
  `{ "tool": "mcp", "pattern": "github_*", "action": "allow" }` covers one
  server.
- The Integrations screen can connect, disconnect and test a server and list
  its tools.

A stdio server is a program running with your user's privileges. Only add
servers you trust.

### Memory on an MCP server

**Settings > Memory > Storage** can point memory at an MCP memory server
instead of the workspace folder, so another coding agent using the same server
shares the same memories. Inertia matches the server's tool names to its
memory operations; if the server cannot be reached it falls back as configured.

## OpenAPI

Import an OpenAPI document (JSON or YAML, from a URL or pasted in) and each
operation becomes a tool. `$ref`s are resolved.

A record, `plugins/openapi/<id>.json`, holds:

- `name` and `source` (where the spec came from, so it can be refreshed);
  the spec itself is kept in `plugins/openapi/specs/`.
- `baseUrl`, to override the `servers` entry in the document.
- `auth`, one of:
  - `{ "kind": "none" }`
  - an API key: `{ "kind": "apiKey", "name": "X-API-Key", "in": "header", "secret": "MY_API_KEY" }`
    (`in` is `header` or `query`)
  - a bearer token: `{ "kind": "bearer", "secret": "MY_TOKEN" }`
- `operations`: which operations to offer as tools. Empty means all of them.

The Integrations screen shows each operation, lets you enable or disable them,
and can call one to check the base URL and key. Operation tools ask under the
`openapi` key with the operation id as the target. An API import that an
agent creates with `inertia_save` has read operations enabled and write
operations disabled.

## Composio

[Composio](https://composio.dev) connects hundreds of hosted apps (Gmail,
Slack, Notion, GitHub, ...) through its own OAuth, and exposes their actions as
tools.

1. Create a Composio API key and save it as the secret `COMPOSIO_API_KEY`.
2. On the Integrations screen, browse the catalogue and connect an app. You
   get a link to sign in with the app's provider; the connection becomes
   active once you do.
3. Choose which of the app's actions agents may use.

Composio tools ask under the `composio` key with the toolkit (app) as the
target. Calls go to Composio's API with your key, and Composio calls the app on
your behalf. The catalogue is cached in `cache/composio-catalogue.json`.
Removing an app with the agent tool `inertia_remove` also revokes the connected account with Composio.

Agents can search the catalogue and start a connection with
`inertia_connect_app`; you still have to open the link and sign in yourself.

## Hooks

A hook is a program (or a short question to the model) that runs at a point in
an agent's turn: before a tool runs, after it runs, or when the agent wants to
stop. The file format is the same as Claude Code's hooks, so existing hook
files work unchanged.

Hooks are read fresh on every turn from, in order:

1. `<workspace>/hooks/hooks.json`: yours, for every agent;
2. `<project>/.inertia/hooks.json`: shipped with a repository; its hooks run
   when that folder is a conversation's working folder;
3. the agent's own record.

**Settings > Hooks** shows what is loaded, any warnings, recent runs, and can
write an example file or switch hooks off.

```json
{
  "enabled": true,
  "PreToolUse": [
    {
      "matcher": "shell",
      "hooks": [
        {
          "name": "no force push",
          "type": "command",
          "command": "node scripts/check-push.js",
          "timeout": 10
        }
      ]
    }
  ],
  "Stop": [
    {
      "hooks": [
        {
          "name": "finish the list",
          "type": "prompt",
          "prompt": "If the task list still has unfinished items the agent never mentioned, block and say which. Otherwise approve."
        }
      ]
    }
  ]
}
```

- **Events that fire**: `PreToolUse` and `PostToolUse` (the matcher is matched
  against the tool name) and `Stop`. The file format also accepts Claude
  Code's other event names (`SessionStart`, `UserPromptSubmit`,
  `PermissionRequest`, `Notification`, `PreCompact`, `PostCompact`,
  `SubagentStart`, `SubagentStop`, `StopFailure`), but Inertia does not fire
  those yet.
- **`command` handlers** receive the event as JSON on stdin. Exit code `0` is
  success; exit code `2` **blocks** (the tool call is denied, or the agent is
  told it may not stop yet), with stderr as the reason; JSON on stdout
  (`decision`, `reason`, `hookSpecificOutput`, `continue`, `systemMessage`) is
  read the way Claude Code reads it. Default timeout 60 s, maximum 600 s.
- **`prompt` handlers** ask the turn's own model, with no tools, to answer
  `{"decision": "approve" | "block", "reason": "..."}`. Default timeout 30 s.
- Handlers for one event run in parallel. The first block wins.
- A hook that crashes or times out is logged and shown as a warning; it never
  ends the turn.
- `"enabled": false` works on a whole file, a group or a single handler.

Hooks run with your user's privileges.

Agents cannot read or change `<workspace>/hooks/` or a project's
`.inertia/hooks.json` with their file tools.
