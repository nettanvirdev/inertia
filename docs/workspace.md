# The workspace folder

Everything Inertia knows lives in one folder you choose on first run:
settings, agents, skills, plugins, conversations, run history, memory and
secrets. It is plain JSON and Markdown, meant to be readable in a file manager
and safe to keep under version control (except `secrets/`, see below).

The default suggestion is `Documents/Inertia` in your home folder. You can pick
any folder, including one that already has files in it; the setup screen tells
you whether it will create a new workspace, add one to a folder that is already
in use, or adopt an existing workspace.

The only thing Inertia stores outside the workspace is a small pointer file,
`workspace.json`, in the platform's app-config directory, which records where
the folder is. Deleting it sends the app back to first-run setup; the folder
itself is untouched.

## Layout

```
<workspace>/
  inertia.json          manifest: marks the folder as a workspace
  README.md             written once, for anyone who opens the folder without the app
  settings/             preferences and configuration documents
  agents/               one JSON file per agent; agents/pictures/ for avatars
  skills/               one folder per skill, each with a SKILL.md
  plugins/
    mcp/                one JSON file per MCP server
    openapi/            one JSON file per imported API
      specs/            the raw spec documents
    composio/           connected Composio apps
  conversations/
    threads/            one file per conversation (title, agent, mode, ...)
    messages/           one file per conversation's transcript
  history/
    sessions/           one record per agent turn: events, tools, token usage
    executions/         tool and routine runs
    activity/           the workspace event log, one file per day
  memory/               one JSON file per memory
  hooks/                hooks.json: lifecycle hooks for every agent
  routines/             one JSON file per routine
  computers/            one JSON file per computer (sandbox or machine)
  files/                attachments and anything an agent produced
    work/               default working folder when no project folder is chosen
    machines/           folders backing "local" computers
  secrets/              secrets.json: API keys and tokens, in plain text
  cache/                re-fetchable data (Composio catalogue, file snapshots). Safe to delete.
  logs/                 diagnostics, including logs/failures. Safe to delete.
  backups/              reserved for snapshots of this folder
```

**Do not rename the directories.** They are the addresses the app resolves
records by. The authoritative list is
`src-tauri/crates/inertia-store/src/layout.rs`.

### settings/

One file per concern, so a hand edit or a merge conflict touches one thing.

| File | Holds |
|---|---|
| `app.json` | app state, such as the default working folder |
| `appearance.json` | reserved for appearance settings |
| `models.json` | model providers and the default model. Names secrets; never contains a key. |
| `permissions.json` | permission rules: `workspace` and per-agent `agents` (see [tools.md](tools.md#permissions)) |
| `identity.json` | your profile as the agents see it, and your preferences: appearance, default mode for new conversations, tool access, ... |
| `group.json` | group-conversation settings |
| `projects.json` | per-folder decisions, such as declining to create an `AGENTS.md` |
| `voice.json` | voice settings |
| `computers.json` | computer provider settings |

`models.json` looks like this:

```json
{
  "providers": [
    {
      "id": "anthropic",
      "name": "Anthropic",
      "enabled": true,
      "baseUrl": "https://api.anthropic.com/v1",
      "kind": "anthropic",
      "apiKeySecret": "ANTHROPIC_API_KEY",
      "models": [{ "id": "claude-sonnet-4", "label": "Claude Sonnet 4" }]
    }
  ],
  "defaultModel": "anthropic/claude-sonnet-4"
}
```

A model is referenced as `<providerId>/<modelId>`; only the first `/`
separates them, so model ids may contain slashes. `kind` is `anthropic` or
`openai`; when absent it is inferred from the base URL.

### agents/

`agents/<id>.json`. Fields include `name`, `role`, `description`,
`systemPrompt`, `model` (`provider/model`), `cwd` (working folder),
`computerId`, `icon`, `avatarColor`, `tags`, thinking settings, delegation
policy, and `protected` for the app's seeded agents. Pictures are copied into
`agents/pictures/` so they travel with the folder.

### skills/

`skills/<slug>/SKILL.md`, with YAML frontmatter, plus any scripts, templates or
reference files the skill needs beside it:

```markdown
---
name: release-notes
description: Write release notes from merged PRs. Use when asked for a changelog or release notes.
---

Instructions the agent reads when it loads the skill...
```

Only `name` and `description` are put in the system prompt; the body is loaded
when an agent calls the `skill` tool. An `allowed-tools` field is passed to the
model as advice and never grants permissions.

### plugins/

- `plugins/mcp/<id>.json`: `name`, `type` (`stdio` or `http`), `enabled`;
  `command`, `args`, `env` for stdio; `url`, `headers` for HTTP; optional
  `timeoutMs`.
- `plugins/openapi/<id>.json`: `name`, `source` (where the spec came from),
  `baseUrl` override, `auth` (`none`, an API key in a header or query
  parameter, or a bearer token, each naming a secret), and which `operations`
  are enabled. The spec itself is kept under `plugins/openapi/specs/`.
- `plugins/composio/`: connected Composio apps and per-app tool permissions.

See [integrations.md](integrations.md).

### conversations/ and history/

Each conversation is `conversations/threads/<id>.json` (metadata) plus
`conversations/messages/<id>.json` (the transcript). Routine runs write into
conversations named `routine-<id>`.

`history/sessions/` keeps one record per agent turn, with its full event log,
so a reloaded window can rejoin a turn that is still running and the History
screen can show what ran.

### memory/

`memory/<id>.json`, one fact per record: `title`, `body`, `kind` (`fact`,
`preference`, `contact`, `project`, `credential-note`, `handover`), `scope`
(`global` or `project`), tags, and bookkeeping such as when it was last
recalled. Project-scoped memories are stored in the project's own
`.inertia/memory/` folder rather than here. Memory can instead be kept by an
MCP memory server, chosen in **Settings > Memory**.

### routines/

`routines/<id>.json`: `name`, `agentId`, `markdown` (the playbook),
`schedule`, `mode`, `enabled`, and the outcome of its last run. See
[agents-and-routines.md](agents-and-routines.md#routines).

### computers/

`computers/<id>.json`: which provider the machine belongs to (`docker`,
`daytona`, `local`), the provider's handle for it, its status and settings. See
[computers.md](computers.md).

### hooks/

`hooks/hooks.json`, in the same format as Claude Code's hooks. See
[integrations.md](integrations.md#hooks).

### secrets/

`secrets/secrets.json`:

```json
{
  "entries": {
    "ANTHROPIC_API_KEY": { "value": "sk-...", "label": "", "createdAt": "...", "updatedAt": "..." }
  }
}
```

Secret names must look like environment variables (letters, digits and
underscores). Everything else in the workspace refers to a secret by name,
either as a field (`apiKeySecret`) or as a `{secret:NAME}` placeholder in an
MCP header or environment variable.

> **Secrets are stored in plain text.** Anyone who can read this file can use
> your keys. Keep the workspace folder private, do not sync `secrets/` to a
> shared or public location, and do not commit it. Encrypted or OS-keychain
> storage is planned. See [security.md](security.md).

## Files Inertia reads from your projects

When a conversation works in a project folder, Inertia also reads (and in some
cases writes) these, relative to the repository root or working folder:

| Path | Purpose |
|---|---|
| `AGENTS.md` | Project instructions, read from the repository root down to the working folder. Inertia offers to create one; it never overwrites an existing file. |
| `.inertia/rules/` | Additional instruction files, read the same way. |
| `.inertia/hooks.json` | Project-level hooks, layered on the workspace hooks. Run when that folder is a conversation's working folder. |
| `.inertia/memory/` | Project-scoped memories. |
| `.inertia/worktrees/<name>` | Git worktrees created by `worktree_enter`. |

## Back up and restore

- **Back up** by copying the whole folder (or committing it to a private
  repository, minus `secrets/`). Close Inertia first, or at least let running
  turns finish, so nothing is mid-write.
- **Restore** by pointing a fresh install at the copy on the setup screen.
  Inertia recognises `inertia.json` and adopts the workspace instead of
  starting over.
- `cache/` and `logs/` can be deleted at any time. File snapshots used for
  per-turn undo live in `cache/snapshots/`; deleting them only removes the
  ability to undo past turns.

**Settings > Workspace** can reveal the folder, switch to a different one
(the old folder is left as it is), or delete everything Inertia manages in it.
Files you put in the folder yourself are never removed.
