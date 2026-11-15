# M0 — Protocol and kernel

- **Goal:** reach the exit criteria of [M0](../spec/12-roadmap.md#m0--protocol-and-kernel):
  a test client creates a thread, sends a message, streams events, disconnects mid-turn,
  reconnects from its cursor and sees no gap or duplicate. Nothing from M1 (no WASM,
  plugin host, WIT, UI).
- **Status:** active (approved 2026-11-15)

## Scope

In:
- Cargo workspace with `kumikit-protocol`, `kumikit-kernel`, `kumikit-core`.
- Protocol types for `core.hello`, `workspace.list/open/close`, `thread.list/create/get/send/cancel/respondApproval/archive`,
  `events.subscribe/unsubscribe`, `events.event`, error codes and every thread event kind
  in [03-kernel](../spec/03-kernel.md#thread-events).
- TypeScript generated into `web/protocol/`, with a staleness test. Recorded JSON fixtures.
- Kernel: domain model, SQLite + migrations, append-only event log, replay from a cursor,
  internal `Provider` trait, turn runner (one turn per thread, queue, cancel, approvals).
- A deterministic scripted provider compiled into the core (deleted in M1).
- `kumikit serve` on the local transport of all three platforms: Unix socket (0600,
  peer-credential check) on Linux and macOS, named pipe (ACL restricted to the user) on
  Windows; NDJSON JSON-RPC, subscriptions.
- Exit-criteria test, footprint measurement, CI.

Out (later milestones or no milestone yet): `auth.*`, pairing, WebSocket/TLS, `provider.*`, `settings.*`, `secret.*`, `plugin.*`, `command.*`,
`core.status/shutdown`, `workspace.files.*` (see Q9), the process supervisor, secret
store, per-plugin KV, tool gateway, `text.delta` coalescing, log compaction.

## Steps

Each step ends with the Verify checks (`sh scripts/check-docs.sh`, then from step 2
`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace`) and one commit.

- [x] **1. Decisions and spec updates (docs only).** Apply the answers to the questions
  below. Files: ADR 0008 (TypeScript generation with ts-rs), ADR 0009 (temporary
  compiled-in scripted provider, an exception to "no provider id in `crates/`"),
  `docs/decisions/README.md` index, `02-protocol.md` (version, casing, subscribe/unsubscribe
  shape, error-code table, handshake rule, batch rule), `03-kernel.md` (approval states,
  restart recovery, durability, derived thread columns), `12-roadmap.md` (provider
  wording: "a deterministic scripted provider … deleted in M1"). Verify: check-docs.
  Commit: `docs: settle M0 protocol and kernel details`.
- [x] **2. Workspace skeleton.** `Cargo.toml` (workspace, resolver 3, `[workspace.package]`
  edition 2024, `license = "MIT"`, `rust-version`; `[workspace.lints]`: `unsafe_code =
  "deny"` (allowed only in the Windows pipe-ACL module, step 10), clippy `unwrap_used`/`expect_used`/`dbg_macro`/`todo` = warn, which
  `-D warnings` turns into errors), `clippy.toml` (`allow-unwrap-in-tests`,
  `allow-expect-in-tests`), `rust-toolchain.toml` (pinned stable + rustfmt, clippy),
  three crates with empty `lib.rs`/`main.rs` and `lints.workspace = true`,
  `kumikit-protocol` with `#![deny(missing_docs)]`. `.github/workflows/ci.yml`: a `rust`
  job (ubuntu, macOS, Windows) running fmt, clippy, test; `deny.toml` + a `cargo-deny`
  licence check (allowlist from AGENTS.md plus the accepted `Unicode-3.0`; SQLite is
  public-domain C source with no crate licence entry). AGENTS.md Verify: move the two Rust
  lines from "From M0" to "Now". Verify: all checks. Commit: `chore: add Cargo workspace and Rust CI`.
- [x] **3. Protocol types.** `crates/kumikit-protocol/src/`: `jsonrpc.rs` (request,
  response, notification, error object, id), `error.rs` (`ErrorCode` enum ↔ stable
  `data.code` strings, numeric JSON-RPC codes), `model.rs` (Workspace, Thread, Turn,
  Approval, statuses, `Id` newtype for ULID strings), `event.rs` (envelope + one typed
  struct per kind, `#[serde(tag = "kind", content = "data")]`), `methods.rs` (a
  `methods!` macro that declares the method table once: name, params type, result type;
  used by core dispatch, TS generation and fixture coverage), `version.rs`. No I/O, deps:
  serde, serde_json, ts-rs. Verify: unit tests for serde shapes (tag names, casing,
  optional fields skipped, unknown fields tolerated). Commit: `feat: add protocol types`.
- [x] **4. TypeScript generation.** `crates/kumikit-protocol/tests/typescript.rs` exports
  every root type with `export_all_to` into a temp dir, plus a hand-written `methods.ts`
  (the method map `{ "thread.create": { params; result } … }` produced from the
  `methods!` table) and `index.ts`, then compares the tree with `web/protocol/src/`.
  Mismatch fails with "run `KUMIKIT_BLESS=1 cargo test -p kumikit-protocol`"; with that
  variable set it rewrites the files. Add minimal `web/protocol/package.json`
  (`@kumikit/protocol`, `license: MIT`, private) and a header comment "generated, do not
  edit" in each file. AGENTS.md Verify: move the stale-TS line to "Now". Verify: test
  passes; editing a generated file makes it fail; changing a Rust type makes it fail.
  Commit: `feat: generate TypeScript protocol types`.
- [x] **5. Contract fixtures.** `crates/kumikit-protocol/tests/fixtures/<method-or-kind>/*.json`,
  each `{ "type": "...", "message": {…} }`. A test parses each into its Rust type,
  re-serializes and compares as `serde_json::Value`; another test fails when a method in
  the table or an event kind has no fixture. Fixtures are first hand-written from the spec
  examples, then replaced by scrubbed recordings from the step 11 test (fixed ULIDs and
  times). Verify: tests; deleting a field from a type breaks a fixture.
  Commit: `test: add protocol contract fixtures`.
- [x] **6. Storage.** `crates/kumikit-kernel/src/store/`: open with WAL,
  `foreign_keys=ON`, `busy_timeout`, `synchronous` per Q12; migration runner over an
  ordered `&[&str]` of embedded SQL files (`migrations/0001_init.sql`), each in its own
  transaction that also sets `user_version`; refuses a database newer than the binary
  (`unsupported`). Schema 1: `workspaces`, `threads` (with derived `status`, `updated`,
  `provider_session`), `turns`, `events (thread, seq, time, kind, data, PRIMARY KEY
  (thread, seq)) WITHOUT ROWID`, `approvals`. No `devices`/`plugins` tables until they are
  used. Concurrency: see Choice C. Verify: tests for fresh DB → latest, upgrade from an
  older version using a synthetic migration list, refusal of a newer DB, WAL on.
  Commit: `feat: add kernel storage and migrations`.
- [x] **7. Domain model and event log.** `Kernel` service with workspaces and threads,
  `append(thread, ThreadEvent) -> seq` (one transaction: event row plus derived columns,
  so `threads.status` can never disagree with the log), `read_after(thread, seq, limit)`,
  a per-thread head `watch::Sender<u64>` bumped after commit, and `thread.get` snapshots
  folded from the log. Injected `Clock` and `IdGen` for deterministic tests. Restart
  recovery per Q8. Verify: tests for seq starting at 1 and contiguous, replay from any
  cursor, snapshot equals fold of the log, recovery of an open turn after reopen.
  Commit: `feat: add kernel domain model and event log`.
- [x] **8. Provider trait and turn runner.** `crates/kumikit-kernel/src/provider.rs`:
  an async trait mirroring the `kumikit:provider` WIT in [05-providers](../spec/05-providers.md)
  (`list_models`, `start_session`, `send`, `respond_approval`, `cancel`, `end_session`) so
  M1 can swap in a WASM adapter. Providers emit through an `EventSink` that accepts only
  provider-emittable kinds (a separate `ProviderEvent` enum, so `turn.started`,
  `turn.ended` and `approval.resolved` cannot be expressed) and stamps the thread and turn
  itself; the kernel assigns approval ids. Turn runner: emits `turn.started`, calls
  `send`, emits `turn.ended`; one running turn per thread, a FIFO queue for further
  messages; cancel; first approval answer wins. Providers are registered by the caller
  (`Kernel::register_provider`), so the kernel names none. Verify: tests with a fake
  provider defined in the test file: event order, a provider error becomes
  `turn.ended { outcome: failed }`, queueing, cancel during an approval, second approval
  answer rejected, sink rejects an event for another turn.
  Commit: `feat: add provider trait and turn runner`.
- [x] **9. Scripted provider.** `crates/kumikit-core/src/scripted_provider.rs`
  (marked `// Temporary: deleted in M1, see ADR 0009`). Models pick a scenario, so tests
  choose behavior through the normal `thread.create { model }`: `echo` (text deltas
  echoing the input, `text.done`, `usage`), `tool-approval` (deltas, `tool.started`,
  `approval.requested`, waits for the decision, then `tool.output`, `tool.finished`,
  `text.done`, `usage`; on deny `tool.finished { ok: false }`), `fail` (one delta, then an
  error). No timers or randomness. Verify: tests driving it through the kernel.
  Commit: `feat: add temporary scripted provider`.
- [x] **10. `kumikit serve`.** `crates/kumikit-core/src/`: `main.rs` (clap: `serve
  [--socket PATH] [--data-dir PATH]`), `paths.rs` (data dir per
  [01-architecture](../spec/01-architecture.md#data-directories), socket path per Q10),
  `transport/unix.rs` (create the socket directory 0700 and verify its owner, refuse a
  live socket, remove a stale one, bind, chmod 0600; per connection compare
  `peer_cred().uid()` with `rustix::process::getuid()`), `transport/windows.rs`
  (`\\.\pipe\kumikit-<user>`, first instance only so a squatter makes `serve` fail,
  remote clients rejected, DACL granting only the current user's SID; the SID lookup and
  security descriptor are the one `unsafe` module, via `windows-sys`). The RPC layer is
  generic over `AsyncRead + AsyncWrite`, so its tests are platform-neutral. `rpc.rs` (NDJSON framing with a
  line-length limit, JSON-RPC dispatch through the method table, handshake rule from Q5),
  `subscription.rs` (Choice D). Prints one `ready <socket>` line on stdout once accepting,
  so tests wait on it instead of sleeping. Graceful shutdown on SIGINT/SIGTERM removes the
  socket. Verify: tests for socket mode 0600, peer check (pure function with a foreign
  uid), parse error, unknown method, call before hello, oversized line.
  Commit: `feat: add kumikit serve on the local socket`.
- [x] **11. Exit-criteria test.** `crates/kumikit-core/tests/exit_m0.rs` with a small
  NDJSON client in `tests/support/`. Spawns `CARGO_BIN_EXE_kumikit serve` with a temp data
  dir and a short temp socket path; waits for `ready`. Client A: hello, `workspace.open`
  on a temp dir, `thread.create { model: "tool-approval" }`, subscribe `after: 0`,
  `thread.send`, reads events until `approval.requested` (turn now paused
  deterministically), records the last `seq`, disconnects. Client B connects, answers the
  approval and reads until `turn.ended`, so events are produced while A is away. A
  reconnects, subscribes `after: <last seq>`, and must receive exactly `last+1 … end`.
  Also: a subscription opened mid-stream gets replay then live with contiguous seq; a
  restart of the core between disconnect and reconnect still replays correctly; the full
  transcript equals `thread.get` folded state. These recordings become the step 5
  fixtures. Verify: `cargo test --workspace`, run 50× in a loop locally to check for
  flakiness. Commit: `test: add M0 exit-criteria test`.
- [x] **12. Footprint.** `scripts/footprint.sh`: builds `--release`, starts `kumikit
  serve` with a fresh temp data dir five times, measures time from spawn to the `ready`
  line (cold start, budget ≤ 300 ms) and, after a hello and one idle second, resident
  memory from `/proc/<pid>/status` `VmRSS` on Linux or `ps -o rss=` on macOS (budget ≤
  30 MB); reports the median and exits non-zero over budget. CI: a `footprint` job on
  ubuntu. Record the first numbers in the Notes below. Commit: `chore: measure core footprint`.
- [ ] **13. Close out.** Spec reflects what was built (02, 03, roadmap M0 marked done),
  non-obvious details moved to `docs/agent-reference.md` and linked from AGENTS.md, AGENTS
  status line updated ("M0 done"), this plan set to `done`. Verify: all checks.
  Commit: `docs: close M0`.

## Choices the spec leaves open

| # | Choice | Options | Recommendation |
|---|---|---|---|
| A | TS generation ([ADR 0005](../decisions/0005-json-rpc-protocol.md) leaves it open) | ts-rs; specta + specta-typescript | **ts-rs**: plain derive, follows serde attributes, stable releases, no runtime registry. Gotcha: it maps `u64` to `bigint`, which `JSON.parse` never produces, so `seq` and token counts get `#[ts(type = "number")]` (values stay below 2^53). **ADR 0008.** |
| B | Staleness check | `#[ts(export)]` side effect in `cargo test`; a compare test with a bless variable | **Compare test** (step 4): a normal test run never writes into the tree, CI fails on drift. |
| C | Async SQLite | rusqlite + `spawn_blocking` per call; tokio-rusqlite; sqlx | **One writer thread** owning a rusqlite connection, fed by an mpsc channel with oneshot replies; it drains queued appends into one transaction (group commit) and is the only place `seq` is assigned. Reads use `spawn_blocking` on a small pool of read-only connections (WAL allows concurrent readers). sqlx is out: the spec chose rusqlite. No ADR: internal, recorded in 03-kernel Storage. |
| D | Live delivery after replay | broadcast channel with payloads + dedupe at the switchover; tail the log | **Tail the log**: each subscription is a cursor reader that reads `seq > cursor` from SQLite whenever the thread's head `watch` moves. Gap- and duplicate-free by construction, a slow client never loses events (it reads from the log, no lagged broadcast), and coalescing can later live in this one reader. Cost: one indexed read per batch per subscriber. |
| E | SQLite build | `bundled` (compiled in); system libsqlite3 | **bundled**: same SQLite version everywhere, no system dependency on macOS/Windows. Licence: see Q13. |
| F | Local auth | `libc`; `nix`; `rustix`; `windows-sys` | Unix: tokio's `UnixStream::peer_cred()` plus **rustix** `getuid()` (safe). Windows: tokio named pipes plus **windows-sys** for the SID and DACL; this needs `unsafe`, so the lint is `deny` with one allowed module. All MIT/Apache. |
| G | Other crates | — | tokio, serde, serde_json, thiserror (libs), anyhow (binary), ulid, time (RFC 3339), tracing + tracing-subscriber, clap; dev: tempfile. All MIT and/or Apache-2.0. Data dirs computed by hand (three platforms, no crate). |
| H | Where the scripted provider lives | in the kernel; a separate crate; in `kumikit-core` | **`kumikit-core`**: the kernel stays name-free, and deleting one file plus one registration line in M1 removes it. Always registered in M0 builds, since it is the only provider. **ADR 0009** records the exception to "no provider id in `crates/`". |
| I | Provider trait shape | free-form; mirror the WIT | **Mirror `kumikit:provider`** so M1 replaces the scripted impl with a WASM adapter behind the same trait. |
| J | Toolchain pin | `channel = "stable"`; pinned version | **Pinned** (`1.99.1`) so clippy results don't change under us; bumped deliberately. Note: on this machine `/usr/bin/cargo` (Arch package) is ahead of rustup on PATH, so the pin applies only in CI and rustup shells. |
| K | Test client | shell/Python script; Rust integration test | **Rust integration test** spawning the real binary: runs under `cargo test`, deterministic, no extra runtime. |

## Questions (unclear or possibly wrong in the spec)

Resolved 2026-11-15: every recommendation below is accepted, except Q10, where all three
platforms ship in M0 (see the resolution under it).

1. **JSON field casing is mixed.** `deviceName`, `respondApproval`, `pluginsChanged` are
   camelCase, but `usage` has `input_tokens`/`output_tokens`. Recommend camelCase for all
   field names (`inputTokens`); enum *values* stay snake_case (`awaiting_approval`,
   `permission_denied`). Fix 03-kernel.
2. **Protocol version.** The spec shows `"1.0"`, but its README says interfaces freeze at
   1.0. Recommend `"0.1"` until the freeze, with the rule that during 0.x a minor bump may
   break (client requires an exact `0.x` match).
3. **Subscribe/unsubscribe shape.** Unspecified: what `events.subscribe` returns and
   whether its response comes before the replayed events; how `events.unsubscribe`
   names a subscription. Recommend: result `{ head: <seq at subscribe time> }` sent
   *before* any `events.event` for that topic; one subscription per topic per connection
   (subscribing again replaces it); `unsubscribe { topic }`.
4. **Other topics.** `workspace:<id>`, `core` and `approvals` have no defined cursor
   semantics (they aren't per-thread logs). Recommend M0 implements `thread:<id>` only;
   other topics return `unsupported`, and their semantics get designed when the UI needs
   them (M2).
5. **Calls before `core.hello`.** On the local socket there is no `auth.*` step. Recommend
   any method other than `core.hello` before it fails with a new code
   `handshake_required`, and a major-version mismatch fails `core.hello` with
   `protocol_mismatch`. Full M0 code list: `invalid_params`, `not_found`,
   `permission_denied`, `unsupported`, `conflict` (e.g. approval already resolved),
   `handshake_required`, `protocol_mismatch`, `plugin_failed`, `internal`. JSON-RPC
   numeric codes: standard `-32700/-32600/-32601/-32602`, `-32000` for the rest, always
   with `data.code`.
6. **`thread.get` snapshot shape** is not defined. Recommend
   `{ thread, turns, pendingApprovals, seq }`; the transcript itself comes from
   subscribing (the log is the record).
7. **Approvals.** Undefined: approval `status` values, the decision wire values, `by` on
   the local socket (there are no devices), and what happens to a pending approval when
   the turn is cancelled. Recommend: status `pending | allowed | denied | cancelled`;
   decisions `allow | allow_always | deny` (from the WIT); `by` is an optional device id,
   absent for local connections; cancel emits `approval.resolved { decision: "cancelled" }`
   before `turn.ended`. Approval ids are assigned by the kernel.
8. **Core restart mid-turn.** Not covered. Recommend: on startup the kernel ends every
   open turn with `turn.ended { outcome: failed, error: "core_restarted" }` (resolving
   its pending approvals as `cancelled` first) and sets the thread `failed`. Real resume
   via the provider's `resume` session id comes with real providers in M1.
9. **`workspace.*` scope.** Recommend `list`, `open { path }` (canonicalized, idempotent:
   the same root returns the same workspace), `close { id }` (hides it from `list`; threads
   and events are kept; reopening the path restores it). Defer `files.list/files.read` to
   M2: not needed for the exit criteria and security-sensitive (symlink escapes), better
   built with the workspace-files service and its tests.
10. **Socket path without `XDG_RUNTIME_DIR`.** macOS has none, and some Linux setups lack
    it. Recommend `$TMPDIR/kumikit-<uid>/core.sock` on macOS and
    `/tmp/kumikit-<uid>/core.sock` as Linux fallback, the directory created 0700 and
    refused if owned by someone else. **Resolved:** Windows ships in M0 too, on the named
    pipe from 02-protocol; CI runs ubuntu, macOS and Windows.
11. **Message queue.** 05-providers says the kernel queues messages sent during a turn,
    but no event records a queued message, so a restart loses it. Recommend in-memory
    FIFO in M0; `thread.send` returns `{ turn, queued }`; persistence later, probably with
    a `turn.queued` event kind (a minor change).
12. **Durability vs cursors.** With WAL and `synchronous=NORMAL`, a power loss can roll
    back events a client has already seen; new events would then reuse those `seq`s and
    the client's cursor would skip them. Recommend `synchronous=FULL` plus group commit
    (Choice C) and publishing the head only after commit; I'll measure append throughput
    and note it. Alternative: `NORMAL` and accept the edge case.
13. **Licences outside the allowed list.** Two transitive inputs aren't on the AGENTS.md
    list: SQLite itself (public domain, compiled in with `bundled`) and `Unicode-3.0`
    (via `unicode-ident`, under every derive macro, so effectively unavoidable).
    **Resolved:** both accepted. I'd also add `cargo-deny` with a `deny.toml` licence allowlist as a CI
    check, so this stays enforced as dependencies grow.
14. **JSON-RPC batches.** Recommend not supported: a batch array gets a single
    `-32600 invalid_request` error. Noted in 02-protocol.
15. **Archive has no thread event.** `status: archived` exists but no event kind records
    it, so "the log is the record" breaks for archiving. Recommend `thread.archive` only
    updates the `threads` row and emits nothing in M0 (it isn't thread content), and the
    spec says so; a later `workspace:` topic event announces it. Alternatively add a
    kernel kind `thread.archived`.
16. **`turn.started.input` attachments** have no defined type. Recommend M0 defines
    `input: { text }` only; `attachments` is added later as an optional field (minor).
17. **`core.hello` plugins.** The scripted provider is not a plugin, so M0 returns
    `plugins: []` and `capabilities: ["threads", "approvals"]`. `thread.create` takes
    optional `provider` and `model`; with one provider registered, `provider` defaults to
    it and `model` to its first model. No `provider.*` methods in M0.

## Notes

- The roadmap's "one provider compiled into the core" becomes "a deterministic scripted
  provider", deleted in M1 together with ADR 0009's exception (step 1 updates the
  wording; M1's existing "The M0 compiled-in provider is deleted" already covers removal).
- Unix socket paths are limited to about 104–108 bytes; tests use a short temp dir
  (`/tmp/kk-XXXX/s.sock`), not the long per-test temp path macOS gives.
- Only `x86_64-unknown-linux-gnu` is installed locally (Arch rust, no rustup
  toolchain), so macOS and Windows code is compile-checked only in CI.
- Step 4: ts-rs 12 has `Config::with_large_int("number")`, so no per-field `u64`
  annotations are needed (ADR 0008 corrected). `web/protocol/` is fully generated,
  including `package.json` and `tsconfig.json`; CI type-checks it with TypeScript 7.0.2.
- Gotcha: clippy's `allow-unwrap-in-tests` does not cover helper functions in
  integration-test crates (`tests/*.rs`); those files carry a crate-level
  `#![allow(clippy::unwrap_used, clippy::expect_used)]`.
- Step 10: the Windows pipe code (`transport/windows.rs`) is written against the
  windows-sys 0.61 and tokio 1.54 signatures read from their sources, but it has not
  been compiled here; the first Windows CI run is its first build.
- Gotcha: tempfile creates directories with the umask's mode (0755 here), and the core
  refuses non-private socket and data directories, so tests chmod their temp dirs to 0700.
- Step 11: the exit tests passed 50 of 50 local runs. Scrubbed recordings replaced the
  first entry of 17 fixture files (provenance in the fixtures README).
- Step 12: written as an `#[ignore]`d Rust test (`crates/kumikit-core/tests/footprint.rs`)
  wrapped by `scripts/footprint.sh`, not a shell script, so timing and process handling
  are portable. First numbers (Linux x86_64, release, no plugins): cold start to `ready`
  2 ms (budget 300 ms), idle RSS 7.1 MB (budget 30 MB), release binary 4.2 MiB. Windows
  RSS is not measured yet.
