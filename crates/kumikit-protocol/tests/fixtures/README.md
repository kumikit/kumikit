# Protocol fixtures

Contract tests for the wire format, run by [`../integration/fixtures.rs`](../integration/fixtures.rs). Each file
is a JSON array:

- `methods/<method>.json`: requests and their responses (success or error).
- `events/<kind>.json`: `events.event` notifications of one kind.
- `notifications/<name>.json`: other notifications the core sends (`core.pluginsChanged`).

A change that makes a fixture fail to round-trip is a breaking protocol change
([versioning](../../../../docs/spec/02-protocol.md#versioning)).

## Where they come from

The first entry of most files is a recording of a real session: the M0 exit test
(`crates/kumikit-core/tests/exit_m0.rs`) writes client transcripts when run with
`KUMIKIT_RECORD=<dir>`. Recordings are scrubbed before they land here:

- ULIDs become stable ids (`01JCWS…1` workspace, `01JCTH…` thread, `01JCTU…` turn,
  `01JCAP…` approval) and timestamps become `2026-11-15T10:mm:ss.000Z` in order.
- The workspace root becomes `/home/user/proj`, the host name `devbox`, and the M0
  scripted provider's id `test.example/example` (fixtures outlive it; provider ids are
  qualified, `<plugin id>/<local id>`).
- Nothing personal remains: no user names, host names or real paths.

Error cases and kinds the scripted provider never emits (`reasoning.delta`,
`file.changed`, `plugin.event`), and the `plugin.*` methods and `core.pluginsChanged`
(added in protocol 0.2), and `provider.*`, `core.shutdown` and the workspace topic events
(added in 0.3), and the `auth.*` methods and `plugin.install { package }` (added in 0.4), and the
`registry.*` methods, `plugin.updates`, `plugin.update`, `plugin.lock`, `plugin.install
{ registry }`, publishers and the `untrusted` and `unavailable` errors (added in 0.5),
and the `secret.*` and `settings.*` methods (added in 0.6), and the `thread.queue.*` methods
and `queue.*` events (added in 0.7; queue entry ids are `01JCQE…`), and `thread.steer` and
`input.steered` (added in 0.8), and `thread.switched` and `handoff.delivered` (added in 0.9),
are written by hand from the
spec. Secret values in them are made up. Device ids are `01JCDV…`; tokens and codes are made up.
