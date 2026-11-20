# 0022 — WIT 0.2.0, and the host drops 0.1.0

- **Status:** Proposed
- **Date:** 2026-11-19

## Context

M5's plugins need what WIT 0.1.0 lacks: plugin methods and commands (`handle-rpc` and
`run-command` in [ADR 0020](0020-interface-freeze-after-m5.md)'s list), plugin settings,
timers, a client interface for bots and schedules, a tool world, outgoing HTTP and a typed
MCP endpoint for providers. A published WIT version is never changed in place, so these
need new versions.

The spec says the host supports the previous major version for at least one kernel major
release. Before 1.0 every minor version is a major one. Keeping 0.1.0 linked beside 0.2.0
means two sets of bindings and glue in the host for every interface, at a time when the
core binary has little headroom. Nothing was published for 0.1.0: the official registry has
no plugins, early publishers accepted breaking changes (ADR 0020), and the only plugins
built for it are this repository's own.

One problem made dropping 0.1.0 hard: bundled plugins are seeded once per id, so an
install that has `provider-acp` built for 0.1.0 would never get the rebuilt one.

## Decision

- New versions: `kumikit:host@0.2.0`, `kumikit:plugin@0.2.0`, `kumikit:provider@0.2.0`,
  and new packages `kumikit:tool@0.1.0` and `kumikit:bot@0.1.0`. They grow during M5 on
  its branch and are recorded in `wit/PUBLISHED` when M5 merges. The 0.1.0 directories
  stay as they are.
- `kumikit:host@0.2.0` keeps the 0.1.0 interfaces unchanged and adds `settings`, `timers`
  and `client`. Outgoing HTTP is `wasi:http` ([ADR 0023](0023-outgoing-http-for-plugins.md)),
  not a Kumikit interface.
- `kumikit:plugin@0.2.0` adds exports `rpc`, `commands` and `timers`, which every world
  (base and provider) exports. WIT cannot make an export optional, so the Rust SDK's
  `Plugin` trait gives each a default that answers `not-found`; a plugin overrides what
  its manifest declares.
- `kumikit:provider@0.2.0` replaces `session-config.mcp-endpoint` (a string) with
  `mcp-server: option<mcp-server>` (`name`, `command`, `args`, `env`).
- The kernel plugin API becomes 0.2 (`api = "^0.2"`). **The host links only 0.2.0
  worlds**: a plugin built for 0.1.0 is refused at enable with a message that it must be
  rebuilt.
- **Bundled plugins update themselves:** at start, a bundled package whose version is
  higher than the installed copy that came from the bundle replaces it, keeping the grant
  when it asks for no more (otherwise it is installed without a grant and waits for one).

## Consequences

- One set of host bindings; the 0.1.0 directories are history.
- Every plugin in this repository and the core template are rebuilt for 0.2.0.
- An early publisher's 0.1.0 plugin stops loading until rebuilt.
- 1.0.0 (after M5) is again a single version; whether the host then keeps 0.2.0 is
  decided at the freeze.
