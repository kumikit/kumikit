# 0012 — Activation memory budget of 8 MB, keeping the pooling allocator

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

## Decision

- Keep the pooling allocator and wasmtime's compile cache.
- The activation budget becomes **≤ 50 ms and ≤ 8 MB**, measured as the first
  activation in a core (it includes creating the engine).

## Consequences

- No extra `unsafe` code; the runtime stays as specified in 04-plugin-host.
- A core with one active plugin uses about 22 MB, still within the 30 MB idle budget's
  spirit; each further instance costs less (the engine and linker are shared).
- If activation memory becomes a problem, the measured alternatives above are the
  starting point; `scripts/footprint.sh` checks the budget.
