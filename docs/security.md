# Security model

Inertia is a single-user desktop app. It runs AI agents on your computer, with
your user account's privileges, using API keys you provide. This page explains
what that means, what protects you, and where the limits are.

To report a vulnerability, see [SECURITY.md](../SECURITY.md).

## The short version

- There is no Inertia server and no account. Nothing is sent anywhere except
  to the model providers and services you configure.
- **API keys are stored in plain text** in `<workspace>/secrets/secrets.json`.
  Keep the workspace folder private.
- Agents act through tools. Every tool call is checked against your
  permission rules, and anything not explicitly allowed asks you first.
- The permission system is **consent, not containment**. A shell command you
  approve runs as you, with access to everything you can access. Only the
  Docker and Daytona computers are sandboxes.

## What is stored, and where

Everything lives in the workspace folder you chose (see
[workspace.md](workspace.md)): conversations, transcripts, tool output, turn
records, memories, agents, settings and secrets, as plain JSON and Markdown.
Outside it, Inertia keeps only a pointer file in the app-config directory that
records where the folder is, plus the webview's own local storage.

### Secrets are plain text

API keys and tokens are stored unencrypted in `secrets/secrets.json`. Anyone or
anything that can read that file can use your keys. Until keychain storage is
added (it is planned):

- keep the workspace folder in your user profile, not in a shared or
  world-readable location;
- do not sync `secrets/` to cloud storage you do not control, and never commit
  it to a repository (exclude it if you version the workspace);
- prefer keys with limited scope and spending limits where providers offer
  them, and rotate a key if the folder may have been exposed.

The rest of the workspace refers to secrets by name only (`apiKeySecret`,
`{secret:NAME}`), so settings, MCP and OpenAPI records can be shared without
leaking keys. In memory, keys are held in a type that cannot be printed by
accident.

## What leaves your machine

There is **no telemetry**: no analytics, no crash reporting, no update check,
no account. Outbound traffic happens only for features you use:

| Destination | What is sent | When |
|---|---|---|
| Your model provider (Anthropic, OpenAI, OpenRouter, a local server, ...) | the system prompt (agent instructions, project `AGENTS.md` and rules, skill descriptions, relevant memories), the conversation, attachments, and tool results, which can include file contents and command output | every turn, title generation and conversation compaction |
| MCP servers | tool arguments; stdio servers are local programs, HTTP servers are remote | when an agent calls their tools |
| APIs imported via OpenAPI | the request the agent builds, with your configured credential | when an agent calls an operation |
| Composio | tool calls with your Composio key; Composio then calls the connected app | when an agent calls a Composio tool; catalogue and logos when you browse |
| ElevenLabs | text to speak, audio to transcribe | when you use voice |
| Daytona | sandbox management, commands, files | when you use a Daytona computer |
| Any website | normal browsing traffic | when you or an agent use the browser pane or a computer's browser |
| Image hosts | an image request | when you click to load a remote image shown in a reply |
| Docker Hub, the Debian archive, nodejs.org | the base image (pinned by digest), Debian packages, the Node.js tarball (checked by SHA-256) | when the Docker sandbox image is built |

Choose providers whose data handling you accept: a turn can send the contents
of any file the agent read.

## The permission system

Every tool call names a permission key and a target (a path, a command, a URL)
and is checked against your rules before it runs. The details are in
[tools.md](tools.md#permissions). In summary:

- **No rule means ask.** Default rules allow reading and searching, and ask for
  changing files, running commands, using the browser pane or a computer,
  integrations, and reaching outside the working folder.
- **Deny is final.** When the deciding rule is a deny, the call is refused
  without asking, and no approval card can turn it into an allow. (Rules are
  ranked by specificity, so a narrower allow can carve an exception out of a
  broad deny; write deny rules as specifically as the thing you want to stop.)
- **No answer is no.** A dismissed card, a cancelled turn or a closed window
  refuses the call.
- **Modes withhold tools.** Chat and Plan mode do not give the model any tool
  that changes something, including delegation tools, so a restricted turn
  cannot ask a helper to act for it.
- **Shell rules see every segment.** "Always allow" on a command saves a
  prefix rule such as `git status *` (which also matches bare `git status`). A
  chained command (`a && b`, `a; b`, `a | b`) is allowed only if every segment
  is allowed, and a deny on any segment denies it. Command or process
  substitution, multiple lines, unclosed quotes and output redirected into a
  real file always ask unless the shell is allowed outright (`shell *`).
- **Some things always ask.** Deleting or emptying a folder
  (`delete_everything`), a shell command that names the secrets folder, and
  agent changes to what an MCP server runs or connects to always show a card,
  with no "Always allow".
- **Some places are off limits.** Agent file tools (and the shell's working
  folder) are refused `<workspace>/secrets/`, `settings/permissions.json`,
  `hooks/` and any project's `.inertia/hooks.json`, whatever the rules say, so
  an agent cannot read your keys, grant itself permissions, or install a hook
  through them. The window's own workspace commands also refuse the secrets
  file; secret values move only through the dedicated secret commands.
- **Outside the working folder asks separately.** File tools and shell
  commands reaching outside the conversation's working folder (and outside
  `<workspace>/files/`) ask under `external_directory`, once per folder.
  Containment is checked on canonical paths, so symlinks and junctions cannot
  escape it.
- **Rule changes are visible.** When an agent asks to change permission
  rules (`inertia_set_rules`), the card lists every rule. Creating an MCP
  server, or changing its type, command, arguments, folder, environment, URL or
  headers, asks under `inertia_guarded` and shows the full command line.
- **Opening files is careful.** When an agent asks the app to open a path,
  executables, scripts and shortcuts are revealed in the file manager rather
  than run.

### Secrets in records

Saved transcripts and turn records replace the values of your stored secrets
(6 characters or longer) with `[REDACTED]`. The failure log also redacts
strings shaped like common tokens. This only catches values Inertia knows
about: a key you pasted into a chat, or one a tool printed that is not in your
secrets, is stored as written.

### Limits

Be clear about what the permission system is not:

- **Shell commands run with your privileges.** Once you allow a command it can
  do anything your user account can, including reading files elsewhere on
  disk. A shell cannot be fenced the way the file tools are, and the parser
  that splits commands and flags dangerous ones is a heuristic that errs
  towards asking; it never decides that something is safe.
- **Secrets are plain text on disk** (see above).
- **Broad allow rules are broad.** If you add a rule allowing every key
  (`*`) or `inertia_guarded`, MCP server changes and rule changes go through
  without asking.
- **MCP cards show headers as written.** A header value on the card is the
  stored text, typically a `{secret:NAME}` placeholder, not the resolved
  value; check which secret it names.
- **The "local" computer is not a sandbox.** It is a folder and your own shell.
- **MCP servers and hooks are programs.** A stdio MCP server, and every
  command hook, runs as you.
- **Prompt injection is possible.** Web pages, files, tool output and API
  responses can contain instructions aimed at the model. The permission system
  limits what an injected instruction can do without your approval; it does
  not stop the model from being misled. Read approval cards before allowing
  them, especially after the agent has read untrusted content.

## Project hooks

A project's `.inertia/hooks.json` hooks run when that folder is a
conversation's working folder. Agents cannot write that file with their file
tools.

## Routines

Routines run unattended. When a routine's tool call needs approval, the card
appears in the routine's conversation and Inertia shows a system notification.
If nobody answers, the run is stopped after 15 minutes.

## The browser pane

The browser pane is a webview inside Inertia, on your machine, sharing one
cookie store across its tabs. Pages you sign in to there stay signed in, and
an agent driving the pane acts with those sessions. Browser tools ask under
the `browser` key by default; a common choice is to allow `http://localhost*`
and keep asking for everything else.

The pane runs with developer tools available (Tauri's `devtools` feature is
enabled in release builds so the pane's DevTools button works).

## The chat window

- The window runs under a restrictive content-security policy: no remote
  scripts, and images only from the app itself or `data:`/`blob:` URLs.
- **Remote images** in a model's reply are not loaded automatically; you click
  to load one, and Inertia fetches it itself, without the system HTTP proxy.
  Addresses on private, loopback, link-local, carrier-grade NAT, unique-local
  and similar ranges are refused, checked after DNS resolution and again on
  every redirect, so a reply cannot make the app probe your local network.
- **Running a code block** (the Run button on a snippet) needs a second,
  confirming click. Snippets run in the conversation's folder with a two-minute limit.
  This is never available to the model; it is only a button for you.

## Cookie import

You can copy cookies from a browser profile on your machine (Chromium-family
browsers and Firefox) into the browser pane or into a computer's browser, so
an agent does not need your password. This is:

- **user-initiated only**: a button in a dialog, never an agent tool;
- **read-only on the source**: browser files are copied before being read and
  never written to;
- **scoped**: you choose the profile and the sites;
- **a real transfer of your sessions**: anything that can drive that browser
  afterwards can act as you on those sites. Import only what the task needs,
  and remove the computer when you are done.

Cookies protected by Chrome and Edge's app-bound encryption (version 127 and
later) are skipped.

## Computers

- **Docker** machines run as a non-root user with `no-new-privileges` and a
  process limit, each on its own Docker network (`inertia-<id>`) so machines
  cannot reach each other's unpublished ports, with a named volume rather than
  a host folder mount. Ports are published on `127.0.0.1` only. Chromium
  inside runs with `--no-sandbox`: the container is the boundary.
- On Docker Desktop a machine can reach services on your own loopback through
  `host.docker.internal`; this cannot be reliably blocked, so do not leave
  unauthenticated services listening on loopback that a sandboxed agent should
  not touch.
- The **live desktop** (noVNC) requires a random password generated for each
  machine. VNC authentication is weak (DES, 8 characters, 48 random bits
  here): it keeps other local containers and processes out, but it is not a
  strong credential, and anyone with Docker access can read it with
  `docker inspect`. On Docker Engine 28+, other containers can reach a
  machine's published ports by container IP, so the password is what
  protects them.
- Machines created before sandbox image 1.1.0 lack these protections until
  recreated.
- **Daytona** machines run in Daytona's cloud under your account.
- The **local** provider is not isolated at all.

See [computers.md](computers.md).

## Installers

Release builds are not code-signed yet. Windows SmartScreen will warn on
first run. Download only from the project's GitHub Releases page, or build
from source.
