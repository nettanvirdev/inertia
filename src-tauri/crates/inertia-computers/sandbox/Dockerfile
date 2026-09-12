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

FROM debian:bookworm-slim

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

# Node from NodeSource rather than the distribution archive, which ships a
# version old enough that half of npm refuses to install against it.
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

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
# not merely of the page that opened it.
EXPOSE 6080 6081

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["/usr/local/bin/inertia-computer"]
