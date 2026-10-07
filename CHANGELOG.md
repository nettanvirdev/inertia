# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-07

First public release.

### Added

- Chat with agents in Chat, Plan, Autonomous and Agent Group modes, against
  Anthropic or any OpenAI-compatible provider.
- Built-in tools for files, search, shell and background processes, git
  worktrees, language servers, and per-turn snapshots with undo.
- Permission rules (allow, ask, deny) per tool, command or path, for the whole
  workspace or one agent, with an approval card for everything else.
- Agents and sub-agents, a background crew, skills, long-term memory and
  scheduled routines.
- Integrations: MCP servers (stdio and HTTP), OpenAPI specs, Composio apps and
  command hooks.
- Computers: Docker and Daytona sandboxes with a live desktop, or a local
  folder.
- A terminal, a browser pane, a Library of everything agents produced, and
  ElevenLabs voice.
- A themed Windows installer and uninstaller.

### Security

- A Content-Security-Policy for the app window, installer and uninstaller.
- Shell "Always allow" rules apply per command segment; command substitution,
  new lines and redirects into files always ask.
- Agent file tools cannot reach the workspace's secrets, permission rules or
  hooks, and ask before leaving the working folder.
- Remote images in replies load only on click, and never from private or
  loopback addresses.
- Each Docker computer gets its own network and a random screen password.
- Stored secret values are redacted from saved transcripts and turn records.

[Unreleased]: https://github.com/nettanvirdev/inertia/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/nettanvirdev/inertia/releases/tag/v0.1.0
