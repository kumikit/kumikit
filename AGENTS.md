# Kumikit — working notes for agents

Kumikit is a pluggable framework for working with coding agents: a headless Rust **core**
(kernel + WASM plugins) and a thin **client** (Tauri 2 shell + Svelte 5 UI kernel in the
system webview) that can connect to several cores, local or remote. Agent providers,
themes, bots and most UI are plugins.

These are the rules to follow before writing code. They are short on purpose. Detail
lives in [docs/spec/](docs/spec/README.md) (what) and [docs/decisions/](docs/decisions/README.md)
(why). Keep this file under 32 KB (`scripts/check-docs.sh` enforces it); add detail to the
spec, not here.

## Status

M0 (protocol and kernel), M1 (plugin host), M2 (client), M3 (multiple cores), M4
(ecosystem), M5 (plugins), M6 (UI design and usability), M7 (threads: durable queue,
steering, handoff, deferred messages, forks, side threads, composer/transcript polish,
usage limits and turn checks) and M8 (signed application updates, idle-gated auto-install,
`kumikit update` CLI, matching SSH remote core sync, bundled `provider-acp` + `threads` in
`core-plugins/`, AppImage CLI forwarding, and borderless window chrome — `v0.1.0-m8.1`) are
done: `kumikit serve`, the kernel, WASM plugins (WIT
0.2.0 plus `kumikit:tool@0.1.0`, `kumikit:turn@0.1.0`, `kumikit:threads@0.1.0` and
`kumikit:session@0.1.0`) with permissions, secrets, settings, plugin methods, core
commands, outgoing HTTP, client interface, schedules and MCP tool gateway, bundled
`provider-acp` and `threads` core plugins, `kumikit plugin`, the desktop app (Tauri shell,
Svelte UI kernel, UI SDK 0.1.5, ui-chat, ui-settings, theme-default), remote cores over TLS
with pairing and device tokens or through SSH, the browser client served by a core, signed
packages, plugin registries with installs by id, updates and lockfiles, sandboxed UI panels,
`kumikit theme check`, plugin templates, and the external first-party plugin repositories
(`kumikit-plugin-agents`, `kumikit-plugin-git`, `kumikit-plugin-github`,
`kumikit-plugin-schedules`)
([M0 plan](docs/plans/2026-11-15-m0-protocol-and-kernel.md),
[M1 plan](docs/plans/2026-11-15-m1-plugin-host.md),
[M2 plan](docs/plans/2026-11-16-m2-client.md),
[M3 plan](docs/plans/2026-11-17-m3-multiple-cores.md),
[M4 plan](docs/plans/2026-11-18-m4-ecosystem.md),
[M5 plan](docs/plans/2026-11-19-m5-plugins.md),
[M6 plan](docs/plans/2026-11-22-m6-ui-design.md),
[M7 plan](docs/plans/2026-11-24-m7-threads.md),
[M8 plan](docs/plans/2026-11-25-m8-updates-and-remote-sync.md)). Build in milestone order
([roadmap](docs/spec/12-roadmap.md)); do not start a later milestone's code before the
earlier one's exit criteria pass.

## How to work

1. **Read before you change.** Find the area in the index below, read its spec section and
   the ADRs it links. The spec is the source of truth.
2. **Plan multi-step work.** For anything that spans several files or a milestone step,
   write a plan in `docs/plans/` first ([how](docs/plans/README.md)), then follow it and
   tick it off.
3. **Keep the spec true.** A change in behavior updates the spec in the same commit. If
   the code and spec disagree, fix the wrong one; never leave both.
4. **Record decisions.** Choosing a technology, changing a published interface or making
   an exception to a rule here needs an ADR ([how](docs/decisions/README.md)).
5. **Verify before claiming done.** Run the checks below that apply and report failures
   as they are. Do not skip or weaken a test to get green.
6. **Small, focused changes.** One concern per commit; no drive-by refactors.

## Verify

Now:
- `sh scripts/check-docs.sh`: AGENTS.md size, and every relative Markdown link resolves.
- `sh scripts/check-no-keys.sh`: no PEM private key in a tracked file.
- `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace` (builds the test plugins for `wasm32-wasip2`; the target comes
  with `rust-toolchain.toml` under rustup, or a distribution's Rust WASM package)
- `cargo clippy --target wasm32-wasip2 -p kumikit-plugin -p kumikit-provider -p kumikit-tool -p kumikit-test-hello -p kumikit-test-probe -p kumikit-test-client -p kumikit-test-tool -p kumikit-provider-acp -p kumikit-threads -- -D warnings`
- `sh scripts/check-plugin-names.sh`: no plugin id, provider id or backend key in
  `crates/*/src`
- `cargo run -p kumikit-core -- theme check --strict plugins/theme-default`: the
  first-party theme passes the theme checks
- `sh scripts/check-templates.sh`: the plugin templates build, pack, sign, install and
  run (needs the `wasm32-wasip2` target and `npm ci`)
- `cargo deny check licenses bans sources` (CI runs it; locally when dependencies change)
- `cargo test -p kumikit-core --test exit_m1 -- --ignored`: live smoke test of provider-acp
  with real Claude Code (needs `claude-agent-acp` and a Claude login; manual)
- `cargo test -p kumikit-protocol` also fails when generated TypeScript is stale;
  `KUMIKIT_BLESS=1 cargo test -p kumikit-protocol` regenerates `web/protocol/`.
- `npx -p typescript@7.0.2 tsc --noEmit -p web/protocol` when protocol types change.
- `sh scripts/footprint.sh`: core cold start and idle memory against the budgets, with
  remote access off and on (release build; CI runs it on Linux).
- `cargo test -p kumikit-shell --test remote -- --ignored`: live SSH profile through the
  real `ssh` (needs `ssh` access to `KK_SSH_DEST`, default `localhost`, where `kumikit`
  is on the `PATH` and a core runs; manual)

From M2 (npm commands run at the repository root, an npm workspace):
- `npm ci`, then `npm run check` (svelte-check + tsc) and `npm test` (Vitest)
- `node scripts/check-npm-licenses.mjs` when npm dependencies change
- Browser end-to-end: `node scripts/bundle.mjs && cargo build -p kumikit-core -p
  kumikit-web-relay -p kumikit-test-registry-fixture -p kumikit-test-plugins`, then
  `npx playwright test` in `web/e2e` (the relay suites, two cores behind a cuttable
  network, the browser client served by a core, a fixture registry, sandboxed panels,
  the tool gateway)
- Desktop exit tests, M2–M5 (Linux, in the container): `sh scripts/desktop/run.sh sh
  scripts/desktop/e2e.sh` after `node scripts/bundle.mjs`; Windows runs in CI
- Desktop shell memory: `sh scripts/desktop/run.sh sh scripts/desktop/footprint.sh`

## Architecture rules (do not break these)

- **The kernel knows no plugin by name.** No provider, bot, theme or tool id or name in
  `crates/` or `web/kernel/`. A special case for one plugin means the plugin API is
  missing something: add the general API instead. ([ADR 0006](docs/decisions/0006-kernel-owns-domain-model.md))
- **First-party plugins use only public SDKs** (`wit/`, `sdk/rust`, `@kumikit/ui-sdk`,
  `@kumikit/protocol`). No private hooks for them.
- **The protocol is the only client–core contract.** `kumikit-shell` and `web/` never
  depend on `kumikit-kernel`. Respect the dependency rules in
  [01-architecture](docs/spec/01-architecture.md#dependency-rules).
- **The shell has no feature commands.** Features go in the core or in plugins. Its
  commands are the connection bridge and the client plugin store ([ADR 0013](docs/decisions/0013-desktop-shell.md)).
- **Core plugins contribute declarative UI only**; UI code comes from client-installed UI
  plugins. ([ADR 0007](docs/decisions/0007-core-plugins-declarative-ui.md))
- **Default deny.** Every host import checks the plugin's granted permissions. Never add
  a host capability without a permission that gates it.
- **WASM only in the core.** No native plugin loading. Sidecars only via the documented
  escape hatch. ([ADR 0003](docs/decisions/0003-wasm-first-core-plugins.md))
- **Lazy by default.** Nothing instantiates at startup unless its activation says so.
  Keep within the [footprint budgets](docs/spec/00-overview.md#footprint-budgets).

## Interfaces and versioning

- Protocol types live only in `crates/kumikit-protocol`. `web/protocol/` is generated;
  never edit it by hand. JSON field names are camelCase, enum values snake_case.
- Adding a method, optional field or event kind is a protocol minor change; removing or
  changing anything is major and needs an ADR. Update the recorded fixtures with the change.
- WIT packages in `wit/` are versioned. Never change a published version in place: add a
  new version.
- The manifest format, the token set and the slot list are public APIs under the same
  rules: published UI SDK versions live in `web/sdk/api/<version>/`, recorded in
  `web/sdk/api/PUBLISHED` ([ADR 0014](docs/decisions/0014-ui-sdk-versioning.md)).

## Rust

- Stable toolchain, edition 2024, tokio. `clippy -D warnings` clean, `rustfmt` default.
- No `unwrap`/`expect` outside tests and provably infallible spots (comment why). Errors:
  `thiserror` in libraries, `anyhow` only in binaries.
- No blocking calls on async tasks; use `spawn_blocking` for SQLite and file I/O.
- External programs run only through the kernel's process supervisor. One exception: the
  desktop shell starts its own bundled core binary ([ADR 0013](docs/decisions/0013-desktop-shell.md)).
- Every public item in `kumikit-protocol` and `sdk/rust` has a doc comment.

## TypeScript and Svelte

- TypeScript strict, no `any` in exported types. Svelte 5 runes, no legacy stores in new code.
- The public UI SDK is framework-agnostic: no Svelte types in `@kumikit/ui-sdk`.
- No literal colors, radii, font sizes or shadows in UI-kernel components: use theme
  tokens (`--kk-*`, [08-themes](docs/spec/08-themes.md)).
- Input is never gated on animation, plugins or the network; every motion has an instant
  path and respects `prefers-reduced-motion`.
- Enter/Escape handlers in text inputs ignore IME composition (`event.isComposing` or
  keyCode 229).

## Security and privacy

- Kumikit is MIT-licensed ([LICENSE](LICENSE)); every crate and package declares
  `license = "MIT"`. Add a dependency only if its license is compatible (MIT, MIT-0, Apache-2.0,
  BSD, 0BSD, ISC, BlueOak-1.0.0, Zlib, MPL-2.0); ask before adding anything else.

- No telemetry, no accounts, no network traffic the user did not start or opt into. Any
  new automatic request goes into the README privacy section in the same change.
- Secrets never reach the webview or a log; the protocol can set secrets, never read them.
- Never commit secrets, tokens, personal paths or real user data (fixtures are scrubbed).
- Remote access is off by default. See [11-security](docs/spec/11-security.md).

## Tests

- Every change ships with tests for what it changes. Bug fixes start with a failing test.
- Protocol: recorded fixtures replayed in `kumikit-protocol`.
- Providers: recorded real agent output replayed through the adapter; one `#[ignore]`d
  live smoke test per provider.
- Plugin host: a test per permission proving the denied path returns `permission_denied`.
- Tests are deterministic: no real network, no sleeps for timing, temp dirs for data.

## Docs and writing

- Write for a reader who has not seen the conversation: plain words, short sentences,
  concrete examples.
- Spec sections describe behavior, not implementation history.
- Link with relative paths; `scripts/check-docs.sh` checks them.

## Git

- Branch from `main`; commit only when asked. Conventional commit prefixes: `feat:`,
  `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- Commit messages say why, not only what.
- Do not rewrite published history.

## Area index

Area → where the code will live → spec section (read it before touching the area).

| Area | Code | Spec |
|---|---|---|
| Goals, principles, budgets, glossary | — | [00-overview](docs/spec/00-overview.md) |
| Processes, crates, layout | workspace root | [01-architecture](docs/spec/01-architecture.md) |
| Protocol, transports, pairing, events | `crates/kumikit-protocol`, `crates/kumikit-core` | [02-protocol](docs/spec/02-protocol.md) |
| Domain model, storage, secrets, processes, tool gateway | `crates/kumikit-kernel` | [03-kernel](docs/spec/03-kernel.md) |
| WASM host, manifest, permissions, lifecycle | `crates/kumikit-plugin-host`, `wit/`, `sdk/rust` | [04-plugin-host](docs/spec/04-plugin-host.md) |
| Agent providers | `plugins/provider-*` | [05-providers](docs/spec/05-providers.md) |
| Desktop shell, UI kernel, connections, tabs | `crates/kumikit-shell`, `web/kernel` | [06-client](docs/spec/06-client.md) |
| UI plugins, slots, declarative UI | `web/sdk`, `plugins/ui-*` | [07-ui-plugins](docs/spec/07-ui-plugins.md) |
| Themes, tokens | `plugins/theme-*`, `web/kernel` theme engine | [08-themes](docs/spec/08-themes.md) |
| Bots | `plugins/bot-*`, `wit/` bot world | [09-bots](docs/spec/09-bots.md) |
| Packages, registry, lockfiles | `crates/kumikit-core` (CLI), registry repo | [10-distribution](docs/spec/10-distribution.md) |
| Threat model | everywhere | [11-security](docs/spec/11-security.md) |
| Milestones, open questions | — | [12-roadmap](docs/spec/12-roadmap.md) |

Code gotchas, invariants and why some tests exist: [agent reference](docs/agent-reference.md).
When an area gains more of them, add them there (or to the area's spec section).
