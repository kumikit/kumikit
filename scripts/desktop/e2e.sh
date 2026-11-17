#!/bin/sh
# The desktop exit test (web/e2e-desktop) on Linux, under Xvfb. Run it in the desktop
# test container: sh scripts/desktop/run.sh sh scripts/desktop/e2e.sh
# Expects target/bundle from `node scripts/bundle.mjs`.
set -eu
# Absolute paths: the app and the core it starts do not run in this directory.
target=$(cd "${CARGO_TARGET_DIR:-target}" 2>/dev/null && pwd || echo "$PWD/target")
cargo build -q -p kumikit-core -p kumikit-test-plugins
cargo build -q -p kumikit-shell --features test-hooks
agent=$(cd "$(dirname "$(find "$target/kumikit-test" -type f -name fake-acp-agent | head -n 1)")" && pwd)
export KK_APP="$target/debug/kumikit-desktop" KK_CORE_BIN="$target/debug/kumikit"
export KK_FAKE_AGENT_DIR="$agent" KK_BUNDLE="$PWD/target/bundle" KK_ARTIFACTS="$PWD/target/e2e-desktop"
exec timeout 600 xvfb-run -a -s "-screen 0 1280x800x24" node web/e2e-desktop/exit.test.mjs
