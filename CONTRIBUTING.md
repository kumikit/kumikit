# Contributing to Kumikit

Thanks for helping. Milestones M0–M5 are built (core, WebAssembly plugins, tool gateway,
schedules, desktop app, remote cores, browser client, package signing, the official plugin
registry and the first-party starter pack), while interfaces remain 0.x until the 1.0
freeze. Reviews of the [specification](docs/spec/README.md), proposals as ADRs, and bug
reports are all useful.

To build and run it, and for the tools each platform needs, see
[Building from source](docs/building.md).

Code conventions for humans and coding agents are in [AGENTS.md](AGENTS.md). Read it
before your first change.

## Proposing a change

- **Small fixes** (typos, broken links, unclear wording): open a pull request.
- **Design changes**: open an issue first, or a pull request with an ADR in
  `docs/decisions/` and the matching spec change.
- **New plugins** do not belong in this repository unless they are part of the
  preinstalled set; publish them through the registry ([distribution](docs/spec/10-distribution.md)).

## Checks

Run before opening a pull request:

```sh
sh scripts/check-docs.sh
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run check && npm test
```

[AGENTS.md](AGENTS.md#verify) has the full list, including the end-to-end and footprint
checks and when each applies; [Building from source](docs/building.md#tests) has the setup
they need.

## Pull requests

- One concern per pull request, with a description of why.
- Update the spec and docs in the same pull request as the behavior they describe.
- Using a coding agent is welcome; you are responsible for what it wrote, so review it
  and run the checks.

## License

By contributing you agree that your contributions are licensed under the
[MIT License](LICENSE).

## Security

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md), not in
public issues.
