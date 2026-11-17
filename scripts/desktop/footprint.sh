#!/bin/sh
# The desktop shell's memory against its budget (00-overview, ADR 0015): anonymous memory
# of the shell process, excluding WebKitGTK's web and network processes, one second
# after the UI kernel connects to the core; median of 5 runs. Linux only, under Xvfb;
# resident memory is printed too, not gated. Expects target/bundle.
# Usage: sh scripts/desktop/footprint.sh   (CI's footprint job, or the desktop container)
set -eu
budget_kb=$((60 * 1024))
target=${CARGO_TARGET_DIR:-target}
cargo build -q --release -p kumikit-core
cargo build -q --release -p kumikit-shell --features test-hooks
work=$(mktemp -d /tmp/kkfXXXX)
chmod 700 "$work"
trap 'pkill -f "kumikit serve --socket $work" 2>/dev/null || true; rm -rf "$work"' EXIT
export KUMIKIT_UI_DIR="$PWD/target/bundle/ui" KUMIKIT_BUNDLED_UI_PLUGINS="$PWD/target/bundle/ui-plugins"
export KUMIKIT_BUNDLED_CORE_PLUGINS="$PWD/target/bundle/core-plugins" KUMIKIT_CORE_BIN="$target/release/kumikit"
export WEBKIT_DISABLE_DMABUF_RENDERER=1
samples=""
for i in 1 2 3 4 5; do
  export KUMIKIT_SOCKET="$work/s$i.sock" KUMIKIT_CORE_DATA_DIR="$work/core$i" KUMIKIT_CLIENT_DATA_DIR="$work/client$i"
  line=$(xvfb-run -a sh -c '
    "$0" > "$1/out" 2>/dev/null & pid=$!
    for _ in $(seq 1 300); do grep -q ui-connected "$1/out" 2>/dev/null && break; sleep 0.1; done
    sleep 1
    anon=$(awk "/^Anonymous:/ {print \$2}" /proc/$pid/smaps_rollup)
    rss=$(awk "/^Rss:/ {print \$2}" /proc/$pid/smaps_rollup)
    kill $pid; wait $pid 2>/dev/null || true
    echo "$anon $rss"' "$target/release/kumikit-desktop" "$work")
  echo "run $i: anonymous $(echo "$line" | cut -d" " -f1) kB, resident $(echo "$line" | cut -d" " -f2) kB"
  samples="$samples $(echo "$line" | cut -d" " -f1)"
done
median=$(echo $samples | tr " " "\n" | sort -n | sed -n 3p)
echo "desktop shell anonymous memory, median of 5: $median kB (budget $budget_kb kB)"
[ "$median" -le "$budget_kb" ]
