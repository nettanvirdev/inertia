#!/usr/bin/env bash
#
# What is installed into the image beyond the packages the Dockerfile names.
#
# Split out from the Dockerfile so the interesting half is editable without
# touching the layer structure - and so Daytona, which may be handed a base
# image it did not build, can run exactly the same script to catch up.
set -euo pipefail

# uv, because a Python agent that has to wait for pip to resolve a dependency
# tree is an agent the user watches spin.
if ! command -v uv >/dev/null 2>&1; then
  curl -fsSL https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin sh
fi

# A predictable place for anything the app copies in later - skills, a helper
# script, a file an agent was told to work on.
mkdir -p /opt/inertia/bin
chmod 755 /opt/inertia/bin

echo "inertia sandbox bootstrap complete"
