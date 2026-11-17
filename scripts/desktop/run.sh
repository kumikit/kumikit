#!/bin/sh
# Runs a command in the desktop test container, with the repository mounted at /src.
# Builds and the cargo registry go to target/desktop-container, so they do not mix with
# host builds and the files stay owned by the caller.
# Usage: sh scripts/desktop/run.sh <command...>   (from the repository root)
set -eu
image=kumikit-desktop-test
docker build -q -t "$image" scripts/desktop >/dev/null
exec docker run --rm -i \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp/home -e CARGO_TARGET_DIR=/src/target/desktop-container \
  -e CARGO_HOME=/src/target/desktop-container/cargo-home \
  -v "$PWD:/src" \
  "$image" "$@"
