#!/usr/bin/env bash
#
# The machine's first breath.
#
# Kept thin: everything expensive belongs in the image, because a container that
# does work on every start is one that feels broken the first time someone stops
# and starts it. This only does what cannot be baked into a filesystem layer -
# processes, and the registrations that depend on them.
#
# The order matters and is not obvious:
#
#   Xvfb, then WAIT FOR IT. A GUI program launched against an X server that is
#   still starting fails with "cannot open display", and that failure reads
#   exactly like a missing package to whoever finds the log.
#
#   dbus before Chromium, or Chromium logs a page of errors about a session bus
#   and the real failure is somewhere in the middle of it.
#
#   xsetroot before fluxbox, because an unpainted root window screenshots as
#   noise or as a black band down whatever part nothing has drawn over.
#
#   xdg-mime before the browser, so a file opened by the agent and a link opened
#   by a document handler both land in the same window with the same profile.
#
# The browser starts here rather than on demand. An agent that has to launch one
# spends its first two calls discovering it is not running, and a cold Chromium
# takes long enough that the first screenshot after it is of a blank window.
set -uo pipefail

export DISPLAY="${DISPLAY:-:99}"
export HOME="${HOME:-/home/agent}"
GEOMETRY="${SCREEN_GEOMETRY:-1600x900x24}"
DISPLAY_NUM="${DISPLAY#:}"

mkdir -p "$HOME" "$HOME/.local/bin" "$HOME/.config" /tmp/inertia /tmp/.X11-unix /tmp/fluxbox-home
export PATH="$HOME/.local/bin:/usr/local/bin:$PATH"
export NPM_CONFIG_PREFIX="$HOME/.local"

# The working directory may arrive as an empty volume, in which case nothing in
# the image is visible under it.
mkdir -p /workspace
cd /workspace

# A stale lock from a container that was killed rather than stopped.
rm -f "/tmp/.X${DISPLAY_NUM}-lock" "/tmp/.X11-unix/X${DISPLAY_NUM}"

Xvfb "$DISPLAY" -screen 0 "$GEOMETRY" -ac +extension RANDR +render -noreset \
  >/tmp/inertia/xvfb.log 2>&1 &
XVFB_PID=$!

ready=0
for _ in $(seq 1 100); do
  if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then ready=1; break; fi
  sleep 0.1
done
if [[ "$ready" -ne 1 ]]; then
  echo "Xvfb failed to start" >&2
  cat /tmp/inertia/xvfb.log >&2 || true
  exit 1
fi

if command -v dbus-launch >/dev/null 2>&1; then
  eval "$(dbus-launch --sh-syntax)"
fi

xsetroot -solid "#1d1f21" >/dev/null 2>&1 || true

# fluxbox gets its own HOME.
#
# Not tidiness: fluxbox writes its running state back into ~/.fluxbox, and the
# agent's home is a workspace that gets exported, imported and diffed. Window
# positions from the last boot are not the user's work and should not travel
# with it.
mkdir -p /tmp/fluxbox-home/.fluxbox
cp /etc/inertia/fluxbox/init /tmp/fluxbox-home/.fluxbox/init
cp /etc/inertia/fluxbox/apps /tmp/fluxbox-home/.fluxbox/apps 2>/dev/null || true
cp /etc/inertia/fluxbox/menu /tmp/fluxbox-home/.fluxbox/menu 2>/dev/null || true
cat > /tmp/fluxbox-home/.fluxbox/startup <<'STARTUP'
#!/bin/sh
xsetroot -solid "#1d1f21"
exec fluxbox -rc /tmp/fluxbox-home/.fluxbox/init
STARTUP
chmod +x /tmp/fluxbox-home/.fluxbox/startup
HOME=/tmp/fluxbox-home /tmp/fluxbox-home/.fluxbox/startup >/tmp/inertia/fluxbox.log 2>&1 &

# Who opens a link.
#
# Checked rather than assumed: xdg-mime exits 0 having done nothing often
# enough that the only way to know is to ask it back. A machine whose
# `open_path` silently opens nothing is worse than one that fails to start.
register_handler() {
  local mime="$1"
  if ! xdg-mime default inertia-browser.desktop "$mime" >/dev/null 2>&1 \
    || [[ "$(xdg-mime query default "$mime" 2>/dev/null || true)" != "inertia-browser.desktop" ]]; then
    echo "failed to register inertia-browser for $mime" >&2
  fi
}
register_handler x-scheme-handler/http
register_handler x-scheme-handler/https
register_handler text/html
xdg-settings set default-web-browser inertia-browser.desktop >/dev/null 2>&1 || true

# A profile left locked by a container that was killed rather than stopped.
rm -f "$HOME/.browser-profiles/chromium/SingletonLock" \
      "$HOME/.browser-profiles/chromium/SingletonCookie" \
      "$HOME/.browser-profiles/chromium/SingletonSocket"

inertia-browser >/tmp/inertia/browser.log 2>&1 &

browser_up=0
for _ in $(seq 1 40); do
  # Both spellings. xdotool's --class takes a regex, not a case-insensitive
  # flag, and Chromium reports its class capitalised on some builds and not on
  # others - so matching only one spelling reports a browser that is plainly on
  # screen as missing.
  if xdotool search --onlyvisible --class '[Cc]hromium' >/dev/null 2>&1; then browser_up=1; break; fi
  sleep 0.25
done
if [[ "$browser_up" -ne 1 ]]; then
  # Not fatal. A machine with a shell and no browser is still a machine, and
  # the tools say so honestly when asked to open something.
  echo "browser failed to start" >&2
  cat /tmp/inertia/browser.log >&2 || true
fi

# The live screen.
#
# Two servers, not one. 5900 is view-only and 5901 is not, and each gets its own
# websockify so the port a client connects to decides what it may do. The
# alternative - one server plus noVNC's `view_only` URL flag - makes "you are
# only watching" a property of the page rather than of the connection, which is
# a promise the browser could break by accident.
#
# Bound to 0.0.0.0 inside the container and published to 127.0.0.1 on the host,
# so the reachable surface is the loopback interface of the machine the app is
# running on and nothing else. -nopw for the same reason: the boundary is the
# port publish, and a password neither side has to type is theatre.
NOVNC_ROOT=/usr/share/novnc
if [[ -d "$NOVNC_ROOT" ]] && command -v x11vnc >/dev/null 2>&1; then
  x11vnc -display "$DISPLAY" -forever -shared -viewonly -nopw     -listen 127.0.0.1 -rfbport 5900 -xkb -ncache 0 >/tmp/inertia/x11vnc-view.log 2>&1 &
  x11vnc -display "$DISPLAY" -forever -shared -nopw     -listen 127.0.0.1 -rfbport 5901 -xkb -ncache 0 >/tmp/inertia/x11vnc-control.log 2>&1 &
  websockify --heartbeat=30 --web="$NOVNC_ROOT" 0.0.0.0:6080 127.0.0.1:5900     >/tmp/inertia/novnc-view.log 2>&1 &
  websockify --heartbeat=30 --web="$NOVNC_ROOT" 0.0.0.0:6081 127.0.0.1:5901     >/tmp/inertia/novnc-control.log 2>&1 &
  screen_up=0
  for _ in $(seq 1 40); do
    if (echo >/dev/tcp/127.0.0.1/6080) >/dev/null 2>&1; then screen_up=1; break; fi
    sleep 0.25
  done
  # Not fatal, for the same reason a missing browser is not: a machine whose
  # screen cannot be watched can still be driven and read.
  [[ "$screen_up" -eq 1 ]] || echo "live screen failed to start" >&2
  echo "$screen_up" > /tmp/inertia/screen-up
else
  echo 0 > /tmp/inertia/screen-up
fi

# A marker the providers stat to tell "this container is one of ours and it is
# up" from "this container exists". Cheaper and more truthful than parsing
# `docker inspect` for a health state nothing sets.
date -u +%Y-%m-%dT%H:%M:%SZ > /tmp/inertia/started-at
echo "$DISPLAY" > /tmp/inertia/display
echo "$browser_up" > /tmp/inertia/browser-up

# Hold the container open for as long as the screen exists. Exiting when Xvfb
# dies is what makes a broken display show up as a stopped machine rather than
# as a running one that answers nothing.
while kill -0 "$XVFB_PID" 2>/dev/null; do
  sleep 2
done
echo "Xvfb exited" >&2
exit 1
