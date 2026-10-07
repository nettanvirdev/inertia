# The machine an agent drives.
#
# Ported from a working implementation rather than invented, because almost
# every line here is the answer to a failure that is invisible until you look at
# a screenshot and see a dialog where the page should be. The ones worth naming:
#
# **Debian, not Ubuntu.** Ubuntu 24.04 ships `chromium` as a transitional stub
# for a snap, and snapd does not run in a container. The build fails, and the
# error blames the package rather than the packaging.
#
# **fbsetbg is replaced, not configured.** fluxbox tries to set a wallpaper on
# start, finds no wallpaper setter, and reports that by opening an xmessage
# dialog in the middle of the desktop - which is then in the middle of every
# screenshot the agent takes. Replacing fbsetbg with a script that paints a flat
# colour is the fix. Turning the wallpaper off in the config is not: fluxbox
# still runs it.
#
# **xmessage is symlinked to /bin/true.** Belt and braces for the same class of
# problem. Nothing in a container can dismiss a modal dialog, so nothing in a
# container should be able to open one.
#
# **Chromium is a wrapper, not the binary.** See `inertia-browser`: the flags
# are not optional and neither is clearing the crash state, or the machine comes
# back from a stop showing "Chrome didn't shut down correctly" across the page
# the agent is trying to read.
#
# **The base is pinned by digest, the packages are not.** The digest is the
# multi-arch index of `debian:bookworm-slim`, so the same bytes are built from
# on an Intel and an ARM machine and a tag that is re-pointed upstream cannot
# change what this file means. The apt packages below are left at whatever
# bookworm currently ships: they are signed by Debian's archive key, which apt
# checks, and pinning every version would freeze security fixes out of an image
# that is rebuilt on each machine that uses it. Bump the digest deliberately
# (`docker buildx imagetools inspect debian:bookworm-slim`) along with the tag
# in image.json.

FROM debian:bookworm-slim@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587

# One layer, because each of these is needed by something that is needed.
#
# xvfb is the display. fluxbox is why windows have titlebars and open maximized
# instead of stacking at 0,0. xdotool is every click, key and drag. imagemagick
# is `import`, which is the screenshot. xclip is how a page is read as text
# rather than only looked at. x11-xserver-utils is xsetroot, which paints the
# root window so an idle screen is a colour and not uninitialised noise.
# xdg-utils is `xdg-open`, which is how "open this file" finds an application.
# dbus-x11 because Chromium logs a wall of errors without a session bus.
# x11vnc, novnc and websockify are the live screen: x11vnc serves the display
# over RFB, websockify puts that on a WebSocket, and novnc is the client that
# renders it. Screenshots answer "what is on screen"; this answers "let me take
# over", which is what a person wants the moment the agent gets stuck.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      git \
      jq \
      ripgrep \
      python3 \
      build-essential \
      procps \
      bash \
      tini \
      xvfb \
      x11-utils \
      x11-xserver-utils \
      fluxbox \
      xterm \
      xdotool \
      xclip \
      imagemagick \
      chromium \
      dbus-x11 \
      x11vnc \
      novnc \
      websockify \
      xdg-utils \
      fonts-dejavu-core \
      fonts-liberation \
  && rm -rf /var/lib/apt/lists/* \
  # fluxbox's wallpaper setter, replaced by one that cannot fail.
  && printf '%s\n' '#!/bin/sh' 'exec xsetroot -solid "#1d1f21" "$@"' > /usr/bin/fbsetbg \
  && chmod +x /usr/bin/fbsetbg \
  # Nothing may open a dialog nobody can close.
  && ln -sfn /bin/true /usr/bin/xmessage \
  && mkdir -p /etc/inertia/fluxbox

# Node from nodejs.org rather than the distribution archive, which ships a
# version old enough that half of npm refuses to install against it.
#
# The official tarball, checked against a SHA-256 written here, rather than a
# vendor setup script piped into bash: that script runs as root with whatever
# its server says today, and this runs only bytes someone has already looked
# at. To bump, take both sums from
# https://nodejs.org/dist/v<version>/SHASUMS256.txt.
ARG NODE_VERSION=22.23.3
RUN case "$(dpkg --print-architecture)" in \
      amd64) arch=x64;   sum=1084aa36196bba4c3a5e69a1ee388a6e4ff729dad09445fbcd434b28fe3c24af ;; \
      arm64) arch=arm64; sum=5ced2d48d1d7198739b7f86804de0171aefb6823b684b12341d3321afc3cb0b2 ;; \
      *) echo "no Node checksum for $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac \
    && curl -fsSLo /tmp/node.tar.gz "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-${arch}.tar.gz" \
    && echo "${sum}  /tmp/node.tar.gz" | sha256sum -c - \
    && tar -xzf /tmp/node.tar.gz -C /usr/local --strip-components=1 --no-same-owner \
    && rm /tmp/node.tar.gz \
    && node --version

# A non-root user, because an agent that runs as root inside its own machine
# will eventually be handed a command that assumes it is not.
RUN useradd --create-home --shell /bin/bash --uid 1001 agent \
    && mkdir -p /workspace \
    && chown -R agent:agent /workspace

ENV DISPLAY=:99
ENV SCREEN_GEOMETRY=1600x900x24
ENV HOME=/home/agent

# --chmod is explicit because a restrictive host umask would otherwise leave
# these unreadable to the agent user at runtime, and the failure reads as a
# missing file rather than as a mode.
COPY --chmod=755 start.sh /usr/local/bin/inertia-computer
COPY --chmod=755 inertia-browser /usr/local/bin/inertia-browser
COPY --chmod=644 inertia-browser.desktop /usr/share/applications/inertia-browser.desktop
COPY --chmod=644 fluxbox.init /etc/inertia/fluxbox/init
COPY --chmod=644 fluxbox.apps /etc/inertia/fluxbox/apps
COPY --chmod=644 fluxbox.menu /etc/inertia/fluxbox/menu

# Strip CR, so a shebang still works when the build context came from a CRLF
# checkout - which on Windows it always has.
RUN sed -i 's/\r$//' /usr/local/bin/inertia-computer /usr/local/bin/inertia-browser \
      /etc/inertia/fluxbox/init /etc/inertia/fluxbox/apps /etc/inertia/fluxbox/menu

WORKDIR /workspace
USER agent

# 6080 watches, 6081 drives. Two servers rather than one with a client-side
# flag, because "you are only watching" should be true of the connection and
# not merely of the page that opened it. Both ask for the password the app
# hands the container at run time; see start.sh.
EXPOSE 6080 6081

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/usr/local/bin/inertia-computer"]
