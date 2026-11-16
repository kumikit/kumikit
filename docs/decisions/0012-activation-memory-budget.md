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

CI's Ubuntu runner measured 11.7–12.4 MB for the same activation (time 7.5–8.1 ms, so
the compiled code came from the cache), against 7.7 MB locally even when pinned to the
runner's four CPUs. The kernel's breakdown (`/proc/<pid>/smaps_rollup`, printed by the
footprint check) differs in kind, not only in size:

| Added by activation | Local (Arch, kernel 7.2) | CI (Ubuntu runner) |
|---|---|---|
| Resident | 7.8 MB | 11.8 MB |
| Private dirty | 3.5 MB | 11.8 MB |
| Anonymous | 3.4 MB | 1.4 MB |
| Clean pages of the core binary | 4.0 MB | none |

On the runner almost everything added is written memory that the kernel counts as
file-backed, which suggests wasmtime's copy-on-write memory images backed by memfd.
This is not verified.

The core binary grew from 4.2 MiB (M0) to 25.3 MiB with the plugin host: the Cranelift
compiler (about 5 MiB), wasmtime and wasmparser (about 2.7 MiB), jsonschema and its regex
engine (about 1.5 MiB) and unwind tables (1.7 MiB). It compresses to 6.8 MiB (xz) to
9.6 MiB (gzip), so the 20 MB installer budget still looks reachable. `opt-level = "s"`
gives 19.1 MiB at some cost in speed.

## Decision

- Keep the pooling allocator and wasmtime's compile cache.
- The activation budget becomes **≤ 50 ms and ≤ 16 MB**, measured as the first
  activation in a core (it includes creating the engine). That covers the CI runner's
  11.7–12.4 MB with room for run-to-run variation.
- A new budget: the **core binary is ≤ 32 MiB** (release build, uncompressed). The
  installer budget (≤ 20 MB, compressed) stays.

## Consequences

- No extra `unsafe` code; the runtime stays as specified in 04-plugin-host.
- A core with one active plugin uses about 19–23 MB; each further instance costs less
  (the engine and linker are shared).
- If activation memory or binary size becomes a problem, the measured alternatives above
  are the starting point. `scripts/footprint.sh` checks both budgets.
- The CI/local gap is not explained. The footprint check prints the breakdown on Linux,
  so the next change to activation memory can start from it.
