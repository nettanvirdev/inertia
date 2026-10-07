# Security policy

## Supported versions

Security fixes are made for the **latest release** only. Please check that an
issue still occurs on the latest release (or on `master`) before reporting it.

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

Report it privately through GitHub:

1. Go to the repository's
   [Security tab](https://github.com/nettanvirdev/inertia/security).
2. Choose **Report a vulnerability** (GitHub private vulnerability reporting).
3. Describe the problem, the affected version and platform, steps to
   reproduce, and the impact you expect.

You should get an acknowledgement within a week. We will work with you on a
fix and a disclosure date, and credit you in the advisory unless you prefer
not to be named.

## Scope

In scope, for example:

- an agent getting around the permission system: running a tool, command or
  file operation that the rules deny, or that should have asked and did not;
- an agent reading or changing `secrets/`, `settings/permissions.json` or
  `hooks/` through its tools;
- breaking out of a Docker sandbox through something Inertia configures, or
  reaching another machine's live desktop without its password;
- web content, a model reply, an MCP server or an API response causing code
  execution or data leaks in the app beyond what the user approved;
- secrets leaking into logs, transcripts, exported records or network
  requests to the wrong destination;
- vulnerabilities in the installer or uninstaller.

Out of scope, because they are documented limits of the design (see
[docs/security.md](docs/security.md)):

- API keys being readable in `secrets/secrets.json` by anyone with access to
  your files (plain-text storage is a known limitation);
- what an approved shell command, an MCP server you added, or a hook you
  configured can do with your user's privileges;
- the "local" computer provider not being a sandbox;
- a model being persuaded by prompt injection to *ask* for something harmful
  (it is a vulnerability only if it happens without the approval the rules
  require);
- unsigned release builds triggering SmartScreen or Gatekeeper warnings;
- vulnerabilities in third-party services (model providers, Composio,
  Daytona, ElevenLabs) themselves.

If you are unsure whether something is in scope, report it privately anyway.
