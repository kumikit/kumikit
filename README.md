# Kumikit

A small, pluggable framework for working with coding agents.

- **Headless core, thin client.** A Rust core runs agents and holds your threads; the
  desktop app (system webview, no bundled browser) connects to it, and to remote cores on
  your other machines, in tabs side by side.
- **Plugins on both sides.** Agent providers, tools, bots, UI panels and themes are
  plugins. Core plugins run as sandboxed WebAssembly components with permissions you
  approve.
- **You choose what's installed.** A handful of plugins come preinstalled; everything else
  is your choice.
- **Light.** Low memory and CPU use by design.

The name comes from *kumiko*, Japanese woodwork in which a lattice is assembled from many
small pieces without nails.

> **Status:** design phase. The specification is in [docs/spec/](docs/spec/README.md);
> there is no code yet.

## Privacy

Kumikit has no accounts and no telemetry. It makes no network requests on its own. The
only traffic is what you start: agents you run, plugin registry lookups when you browse or
install, and connections to cores you pair with.

## Documentation

- [Specification](docs/spec/README.md): how Kumikit works.
- [Decisions](docs/decisions/README.md): why it works that way.
- [Contributing](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) (rules for humans and coding agents).
- [Security](SECURITY.md).

## License

[MIT](LICENSE).

## Links

- Website: <https://kumikit.dev>
- Source: <https://github.com/kumikit>
