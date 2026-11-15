#!/bin/sh
# Measures the core against its footprint budgets (docs/spec/00-overview.md):
# cold start to accepting connections and idle resident memory, median of 5 runs, on a
# release build. Exits non-zero when over budget.
# Usage: sh scripts/footprint.sh   (run from the repository root)
set -eu
exec cargo test --release -p kumikit-core --test footprint -- --ignored --nocapture
