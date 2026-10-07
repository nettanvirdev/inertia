# Getting started

## Install

### Download a release (Windows)

Download `Inertia-Setup-<version>.exe` from the
[Releases page](https://github.com/nettanvirdev/inertia/releases) and run
it. It installs per user (no administrator prompt) and can add Start menu and
desktop shortcuts. Uninstall from **Settings > Apps** in Windows as usual.

The builds are not code-signed yet, so Windows SmartScreen will show "Windows
protected your PC". Choose **More info > Run anyway** if you trust the
download. If you would rather not, build it yourself.

Requirements: Windows 10 or 11 with the Microsoft Edge WebView2 runtime
(preinstalled on Windows 11 and on current Windows 10).

### Build from source (any platform)

Windows is the primary, tested platform. The code is cross-platform and
`bun run tauri build` produces a macOS `.dmg` or Linux packages, but those
builds are untested. See [development.md](development.md) for prerequisites,
then:

```bash
git clone https://github.com/nettanvirdev/inertia.git
cd inertia
bun install
bun run tauri build
```

The installers end up under `src-tauri/target/release/bundle/`.

## First run: choose a workspace folder

The first screen asks where your workspace folder should go. Everything
Inertia keeps (settings, agents, conversations, memory, secrets) is stored
there as plain files. The suggestion is `Documents/Inertia`.

Below the field, the screen says what it is about to do:

- **create** a new workspace in an empty or new folder,
- **create in a folder already in use**, if the folder has other files in it
  (they are left alone), or
- **adopt** an existing Inertia workspace, if the folder already has one.
  This is also how you restore from a backup or move to a new machine.

You can change the folder later in **Settings > Workspace**. See
[workspace.md](workspace.md) for what goes in it.

## Add a model provider

Inertia does not come with a model or an account. Bring your own API key.

1. Open **Settings > Providers** and add a provider. Presets are offered for
   Anthropic, OpenAI, OpenRouter, Groq, Together, DeepSeek, xAI and Ollama;
   any OpenAI-compatible endpoint works with its base URL.
2. Paste your API key. It is saved as a named secret (for example
   `ANTHROPIC_API_KEY`) in `<workspace>/secrets/secrets.json`, and the provider
   record only stores the name. Local servers such as Ollama need no key.
3. Use **Test connection**, then pick the models you want and set a default
   model.

Anthropic is spoken natively (so prompt caching and extended thinking work);
everything else uses the OpenAI chat-completions protocol.

> API keys are stored in plain text in the workspace folder. Keep that folder
> private. See [security.md](security.md).

## Your first chat

Start a new conversation and type. The **mode**, chosen in the composer,
decides what the agent may do: *Chat* (talk it through; no tools that change
anything), *Plan* (investigate and end with a plan you approve), *Autonomous*
(do the work with full tools), or *Agent Group* (several agents in one
conversation). New conversations start in Chat.

When the agent wants to do something your rules say to ask about, a card
appears showing the exact command or path, with **Allow**, **Always allow**
and **Deny**. See [tools.md](tools.md#permissions).

Useful next steps:

- Set a working folder for the conversation so the agent works in your project.
  Without one it works in `<workspace>/files/work`.
- Create agents with their own instructions, model and permissions in
  **Agents**. See [agents-and-routines.md](agents-and-routines.md).
- Connect MCP servers, OpenAPI specs or Composio apps in **Integrations**. See
  [integrations.md](integrations.md).
- Open the terminal and browser panes beside the conversation; the agent can
  read your terminal and drive the browser pane (with permission).

## Optional: Docker for computers

Agents can be given their own machine with a desktop they can see and drive.
For local sandboxes you need [Docker](https://docs.docker.com/get-docker/)
(Docker Desktop on Windows and macOS). Inertia builds its sandbox image
locally the first time; that takes several minutes. Without Docker you can
still use a Daytona cloud sandbox (needs a Daytona API key) or the "local"
provider, which is a folder on your machine and not a sandbox.

See [computers.md](computers.md).

## Optional: voice

Voice input and spoken replies use ElevenLabs. Add an `ELEVENLABS_API_KEY`
secret and configure it in **Settings > Voice**.

## Running in the background

Closing the window can minimise Inertia to the tray (**Settings > General**),
which keeps routines running on schedule. **Quit Inertia** from the tray icon
stops everything. Inertia can also start when you log in.
