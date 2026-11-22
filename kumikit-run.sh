#!/bin/sh
# Launches the Kumikit desktop app against the local workspace build and preview profile
# (with all first-party plugins installed in /tmp/kk-ui).
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
DATA="${KUMIKIT_PREVIEW_DIR:-/tmp/kk-ui}"
mkdir -p "$DATA/core" "$DATA/client"
chmod 700 "$DATA" "$DATA/core"

export KUMIKIT_SOCKET="${KUMIKIT_SOCKET:-$DATA/s.sock}"
export KUMIKIT_CORE_DATA_DIR="${KUMIKIT_CORE_DATA_DIR:-$DATA/core}"
export KUMIKIT_CLIENT_DATA_DIR="${KUMIKIT_CLIENT_DATA_DIR:-$DATA/client}"
export KUMIKIT_CORE_BIN="${KUMIKIT_CORE_BIN:-$ROOT/target/debug/kumikit}"
export KUMIKIT_UI_DIR="${KUMIKIT_UI_DIR:-$ROOT/target/bundle/ui}"
export KUMIKIT_BUNDLED_UI_PLUGINS="${KUMIKIT_BUNDLED_UI_PLUGINS:-$ROOT/target/bundle/ui-plugins}"
export KUMIKIT_BUNDLED_CORE_PLUGINS="${KUMIKIT_BUNDLED_CORE_PLUGINS:-$ROOT/target/bundle/core-plugins}"
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"

exec "$ROOT/target/debug/kumikit-desktop" "$@"
