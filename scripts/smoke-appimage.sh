#!/bin/sh
# Starts the AppImage under Xvfb and checks that it works end to end: the shell starts
# its bundled core only when the UI kernel's JavaScript asks for a connection, so a core
# that comes up with the bundled provider-acp enabled means WebKit loaded and ran the UI,
# and the AppImage's resources were found. Needs xvfb-run and a kumikit binary for the
# check (any build; it only talks to the core).
# Usage: sh scripts/smoke-appimage.sh <file.AppImage> <kumikit binary>
set -eu
app=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
kumikit=$2
work=$(mktemp -d /tmp/kkaXXXX)
chmod 700 "$work"
cleanup() {
  [ -n "${pid:-}" ] && kill "$pid" 2>/dev/null || true
  pkill -f "serve --socket $work/s.sock" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT
export KUMIKIT_SOCKET="$work/s.sock" KUMIKIT_CORE_DATA_DIR="$work/core" KUMIKIT_CLIENT_DATA_DIR="$work/client"
# No FUSE needed: the runtime extracts itself.
export APPIMAGE_EXTRACT_AND_RUN=1
# CLI arguments forward directly to the bundled kumikit binary without needing a display.
"$app" --help | grep -q "Usage: kumikit"
"$app" --version | grep -q "^kumikit "
xvfb-run -a "$app" > "$work/app.log" 2>&1 &
pid=$!
i=0
until "$kumikit" plugin list --socket "$KUMIKIT_SOCKET" --data-dir "$work/none" 2>/dev/null | grep -q "kumikit.provider-acp .* enabled"; do
  i=$((i + 1))
  if [ "$i" -gt 120 ]; then
    echo "the AppImage did not start its core with provider-acp within 60 s; its output:" >&2
    cat "$work/app.log" >&2
    exit 1
  fi
  sleep 0.5
done
# Verify the AppImage CLI itself can also talk to the running core.
"$app" plugin list --socket "$KUMIKIT_SOCKET" --data-dir "$work/none" | grep -q "kumikit.provider-acp .* enabled"
echo "AppImage smoke test ok: the UI connected, CLI forwarding works, and the bundled core and plugins work"
