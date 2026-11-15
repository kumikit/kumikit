# Contributing to Kumikit

Thanks for helping. Kumikit is in its design phase: the most useful contributions right
now are reviews of the [specification](docs/spec/README.md) and proposals as ADRs.

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
```

More checks are added as code lands; [AGENTS.md](AGENTS.md#verify) has the current list.

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
