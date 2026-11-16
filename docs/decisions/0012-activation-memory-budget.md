# 0012 — Budgets for plugin activation memory and the core binary

- **Status:** Proposed
- **Date:** 2026-11-16

## Context

The footprint budgets ([00-overview](../spec/00-overview.md#footprint-budgets)) allowed
5 MB for activating a typical WASM plugin. The first measurement in M1 (provider-acp,
release build, Linux, a fresh core with a warm compile cache) added 7.5 MB:

| Step | Memory |
|---|---|
| Creating the wasmtime engine | 1.9 MB |
| WASI linker | 0.7 MB |
| Loading the compiled component through wasmtime's cache | 2.3–2.6 MB |
| Instantiating with the pooling allocator | 2.6–2.9 MB |

Two changes together measured about 3.6 MB: the on-demand allocator (instantiation
about 0.9 MB) and loading precompiled code with `Component::deserialize_file`, which maps
the file so only touched pages count (0.2 MB), at the cost of a new `unsafe` module.
Either alone stayed over 5 MB. Fewer pooling slots did not help usefully (8 slots
still cost 2.6 MB).

Most of the cost is paid once per core (engine, linker), not per plugin.

CI's Ubuntu runner measured 11.7 MB for the same activation (time 8.1 ms, so the compiled
code came from the cache), against 7.7 MB locally even when pinned to the runner's four
CPUs. The difference is not explained yet.

The core binary grew from 4.2 MiB (M0) to 25.3 MiB with the plugin host: the Cranelift
compiler (about 5 MiB), wasmtime and wasmparser (about 2.7 MiB), jsonschema and its regex
engine (about 1.5 MiB) and unwind tables (1.7 MiB). It compresses to 6.8 MiB (xz) to
9.6 MiB (gzip), so the 20 MB installer budget still looks reachable. `opt-level = "s"`
gives 19.1 MiB at some cost in speed.

## Decision

- Keep the pooling allocator and wasmtime's compile cache.
- The activation budget becomes **≤ 50 ms and ≤ 12 MB**, measured as the first
  activation in a core (it includes creating the engine), leaving room for the
  difference between machines seen in CI.
- A new budget: the **core binary is ≤ 32 MiB** (release build, uncompressed). The
  installer budget (≤ 20 MB, compressed) stays.

## Consequences

- No extra `unsafe` code; the runtime stays as specified in 04-plugin-host.
- A core with one active plugin uses about 19–23 MB; each further instance costs less
  (the engine and linker are shared).
- If activation memory or binary size becomes a problem, the measured alternatives above
  are the starting point. `scripts/footprint.sh` checks both budgets.
- The CI/local gap should be explained when the footprint check next needs attention
  (for example by printing `/proc/<pid>/smaps_rollup` in the check).
