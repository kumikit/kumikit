<p align="center">
  <img src="assets/brand/kumikit-icon.svg" width="128" height="128" alt="Kumikit icon: an asanoha kumiko pattern in a hexagon">
</p>

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

> **Status:** early, not yet released. The core, WebAssembly plugins, the desktop app,
> remote cores and the browser client are built (milestones M0–M3); the plugin registry
> and package signing (M4) are not. There are no published installers yet:
> [build from source](docs/building.md). The specification is in
> [docs/spec/](docs/spec/README.md).

## Privacy

Kumikit has no accounts and no telemetry. It makes no network requests on its own. The
only traffic is what you start: agents you run, plugin registry lookups when you browse or
install, and connections to cores you pair with.

A core listens on the network only when you start it with `kumikit serve --listen`; it
never turns that on by itself. The desktop app connects only to cores you added, over TLS
or through your own `ssh`, and sends keepalive pings only on those connections, so it
notices when a network goes away.

The desktop app shows web pages only from its own files: links in agent output are copied,
not opened, and markdown images are shown as text, so nothing is fetched from the web. On
Windows the app uses the system's WebView2, whose own diagnostic data follows your Windows
settings; the Windows installer carries the WebView2 bootstrapper, which downloads the
runtime only if it is missing when you install.

## Documentation

- [Building from source](docs/building.md): prerequisites and steps for Linux, macOS and
  Windows.
- [Specification](docs/spec/README.md): how Kumikit works.
- [Decisions](docs/decisions/README.md): why it works that way.
- [Contributing](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) (rules for humans and coding agents).
- [Security](SECURITY.md).

## License

[MIT](LICENSE).

## Links

- Website: <https://kumikit.dev>
- Source: <https://github.com/kumikit>
