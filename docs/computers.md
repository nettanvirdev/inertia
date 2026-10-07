# Computers

A computer is a machine an agent can drive: run commands on, read and write
files on, and, for the sandboxes, see the screen of and click around in.
Computers are managed on the **Computers** screen, assigned to an agent in the
agent editor, and configured in **Settings > Computers**. Each is a record in
`<workspace>/computers/`.

An agent with a computer gets the `computer_*` tools (see
[tools.md](tools.md#computers)). They all ask under the `computer` permission
key, which defaults to `ask`.

## Providers

| Provider | Where it runs | Desktop | Isolation |
|---|---|---|---|
| **Docker** | a container on this machine | yes, with a live view | a container: separate filesystem, network and processes |
| **Daytona** | a sandbox in Daytona's cloud; keeps running when Inertia is closed | yes, screenshots | a remote machine |
| **Local** | a folder on this machine, `<workspace>/files/machines/<id>`, using your own shell | no | **none**: not a sandbox |

### Docker

Needs Docker installed and running; Inertia drives it through the `docker`
CLI, so Docker Desktop, Colima or a remote Docker context all work.

The machine is built from the **sandbox image** in
`src-tauri/crates/inertia-computers/sandbox/`. Its files are compiled into
the Inertia binary, and the image, `inertia-sandbox:1.1.0`, is always built
locally: the first time you make a Docker computer, or earlier with **Build
image** in **Settings > Computers**, or by hand:

```bash
docker build -t inertia-sandbox:1.1.0 src-tauri/crates/inertia-computers/sandbox
```

Nothing is pulled from an image registry. A first build takes several
minutes. The base image (`debian:bookworm-slim`) is pinned by digest and
Node.js 22 comes from the official tarball, verified by SHA-256; Debian
packages come from the signed bookworm archive and follow its updates (they
are not version-pinned).

The image is Debian 12 with:

- a virtual display (Xvfb, 1600x900) with the fluxbox window manager,
- Chromium, wrapped so it starts cleanly after a stop (it runs with
  `--no-sandbox`; the container is the boundary),
- `xdotool` for clicks and keys and ImageMagick `import` for screenshots,
- git, Python 3, Node.js, build-essential, ripgrep, jq and curl,
- x11vnc, websockify and noVNC for the live view,
- a non-root user `agent` (uid 1001) and a working folder `/workspace`.

Each Docker machine:

- runs as the non-root `agent` user with `--security-opt no-new-privileges`
  and `--pids-limit 4096`, plus the CPU and memory limits you set when
  creating it (none if you set none);
- gets **its own Docker network**, `inertia-<id>`, so machines cannot reach
  each other's unpublished ports. Docker's default address pools allow about
  30 networks in total; past that, creating a machine fails with a message
  suggesting `docker network prune`;
- publishes its ports to `127.0.0.1` only;
- receives environment variables for commands through an `--env-file`, not
  on the command line;
- keeps `/workspace` on a named volume, `<container>-workspace`, so its files
  survive the container being removed; no host folder is mounted;
- can reach services on your machine's loopback through
  `host.docker.internal` on Docker Desktop (useful for a dev server you are
  running; this cannot be reliably blocked);
- can be snapshotted (`docker commit`, labelled with the machine it belongs
  to) and restored. A restore starts the machine with the same CPU and memory
  limits and a new screen password, and its desktop comes back.

Machines created before image 1.1.0 keep working but lack these protections
(they share Docker's default bridge network and have no screen password) until
you recreate them. Snapshots taken before 1.1.0 are not listed.

### Daytona

Needs a [Daytona](https://daytona.io) account. Save your API key as the
secret `DAYTONA_API_KEY`; for a self-hosted Daytona, also set the secret
`DAYTONA_API_URL`.

Daytona sandboxes are created from a **snapshot** on your Daytona account. By
default Inertia asks for one named `inertia-sandbox-1.1.0`, matching the image
version in `sandbox/image.json`. You create that snapshot once, on your own
account, from the same image:

1. Build the image locally, with **Build image** in Settings or:
   ```bash
   docker build -t inertia-sandbox:1.1.0 src-tauri/crates/inertia-computers/sandbox
   ```
   On Apple Silicon, add `--platform linux/amd64`.
2. Push it to Daytona as a snapshot with exactly that name, for example:
   ```bash
   daytona snapshot push inertia-sandbox:1.1.0 --name inertia-sandbox-1.1.0
   ```
   or create the snapshot from a registry image in the Daytona dashboard.
   The CLI changes over time; see Daytona's documentation for the current
   commands.

Any other snapshot on your account can be picked in the **New computer**
dialog. If the snapshot is missing, creating a computer fails with an error
pointing here.

Daytona sandboxes keep running when Inertia is closed and cost money while
they run. **Settings > Computers** can stop an idle cloud sandbox after a set
time.

### Local

The local provider is a folder in the workspace and your own shell. It works
offline with nothing installed, and you can inspect what the agent did in your
file manager. **It is not a sandbox**: commands run as you and can reach
anything you can. Relative paths are kept inside the machine's folder, but an
absolute path is followed. Treat it like the `shell` tool, and rely on the
permission rules.

## The live desktop

For Docker machines, the **Computers** screen and the side pane in a
conversation show the machine's screen live through noVNC, served from inside
the container:

- two servers: port 6080 is view-only and 6081 accepts input, so "watching" is
  a property of the connection rather than of the page;
- published to `127.0.0.1` only, on a port Docker picks, so the screen is not
  exposed to your network;
- protected by a **random password generated for each machine** when it is
  created or restored. It is passed to the container in its environment and
  written to a file only the container user can read; the app supplies it
  automatically. Without a password the live screen is not served at all.

Limits of that password: VNC authentication is DES-based and uses at most 8
characters (48 random bits here). It keeps other local containers and
processes out; it is not a strong credential. Anyone with access to Docker can
read it with `docker inspect`. On Docker Engine 28 and later, other containers
can still reach a machine's published ports by its container IP, so the
password is what protects them.

You can take over the mouse and keyboard at any point. Daytona machines show
still screenshots instead of a live stream. Local machines have no screen.

## Signing in on a computer

Agents start signed in to nothing. Rather than typing passwords, you can copy
your existing session cookies into a machine's browser (or into the browser
pane) from a browser profile on your machine: Chromium-family browsers (Chrome,
Edge, Brave, ...) and Firefox. This is a button in the app, never an agent
tool, and you choose the profile and the sites. Chrome and Edge 127+ protect
some cookies with app-bound encryption; those are skipped rather than guessed
at. See [security.md](security.md#cookie-import).

## Choosing between a computer and the shell

- Use the **`shell` tool** (no computer needed) for work on your own files and
  projects.
- Use a **Docker or Daytona computer** for anything that should not touch your
  machine: installing packages, running untrusted code, long builds, or
  browsing as a separate identity.
- The **browser pane** is your own browser on your machine (your cookies, your
  localhost); `computer_open` opens pages inside the sandbox, which cannot see
  your localhost (except through `host.docker.internal` on Docker Desktop) and
  has none of your logins unless you import them.

## Changing the sandbox image

Edit the files in `src-tauri/crates/inertia-computers/sandbox/` and bump `tag`
(and `daytonaSnapshot`) in `image.json`. A new tag makes Inertia build a fresh
image; existing machines keep running on the old one until recreated. Files in
that folder must keep LF line endings (`.gitattributes` enforces this).
