# Building Kumikit from source

How to build and run Kumikit on Linux, macOS and Windows: the core (`kumikit`), the
desktop app (`kumikit-desktop`), the browser client, and the installers. The commands are
the ones CI runs ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)); the checks to
run before a pull request are in [AGENTS.md](../AGENTS.md#verify).

Commands are run from the repository root. On Windows, run the `sh` scripts and the
`cargo`/`npm` commands from Git Bash (CI does the same); PowerShell works for `cargo` and
`npm` alone.

## What you need

### Every platform

| Tool | Version | Why |
|---|---|---|
| [rustup](https://rustup.rs) | any recent | Installs the Rust toolchain pinned in [`rust-toolchain.toml`](../rust-toolchain.toml) (1.100.0 with `rustfmt`, `clippy` and the `wasm32-wasip2` target) the first time you run `cargo` here. A distribution's `cargo` without rustup also works if it is 1.100 or newer and has the `wasm32-wasip2` target. |
| [Node.js](https://nodejs.org) with npm | 24 or newer | The UI kernel, the UI SDK, the first-party UI plugins and the browser tests (an npm workspace at the root). |
| Git | any | |
| A C compiler | | The TLS library (`ring`) and SQLite are built from C. Per platform below. |

The core alone (`cargo build -p kumikit-core`) needs only Rust and a C compiler. Node is
needed for the desktop app, the browser client and anything that runs
`node scripts/bundle.mjs`.

### Linux

The desktop shell uses the system's WebKitGTK 4.1. On Debian or Ubuntu 22.04 and newer
(what CI uses):

```sh
sudo apt-get install build-essential pkg-config \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libsoup-3.0-dev
```

On Arch Linux: `sudo pacman -S --needed base-devel webkit2gtk-4.1 gtk3 librsvg libsoup3`.
On other distributions install the same libraries' development packages; Tauri's
[prerequisites page](https://v2.tauri.app/start/prerequisites/) lists their names (Kumikit
needs no tray or app-indicator library).

Only for some tasks:

- Linux installers: `rpm squashfs-tools file patchelf` (Debian/Ubuntu names).
- The desktop end-to-end tests and the shell's memory check run in a container: install
  [Docker](https://docs.docker.com/engine/install/). The image
  ([`scripts/desktop/Dockerfile`](../scripts/desktop/Dockerfile)) brings WebKitGTK's
  WebDriver, Xvfb and the rest.
- Playwright's WebKit needs `libflite` on some distributions (Arch has none packaged; there,
  run the browser tests with `--project chromium` and leave WebKit to CI).

### macOS

- Xcode Command Line Tools: `xcode-select --install`.
- The desktop app uses the system's WKWebView; nothing else to install.

macOS has no WebDriver for WKWebView, so the desktop end-to-end tests do not run there; the
Playwright suites (Chromium and WebKit) cover the same flows.

### Windows

- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  with the **Desktop development with C++** workload (the MSVC linker and the Windows SDK).
  Install Rust's `x86_64-pc-windows-msvc` toolchain (rustup's default on Windows).
- The [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/), which
  Windows 10 and 11 already have.
- [Git for Windows](https://gitforwindows.org), for Git Bash.

For the desktop end-to-end tests: `cargo install --locked tauri-driver --version 2.1.0`
and an `msedgedriver` matching the installed WebView2 version (see the `e2e-desktop` job in
CI for how it is fetched).

## Build and run

### 1. Dependencies

```sh
npm ci
```

### 2. The core

```sh
cargo build -p kumikit-core              # target/debug/kumikit
cargo run -p kumikit-core -- serve       # a core on this machine's local socket or pipe
```

`kumikit serve` prints `ready <address>` once it accepts connections and keeps your data
in the platform's data directory ([01-architecture](spec/01-architecture.md#data-directories));
`--data-dir` and `--socket` move both, which is handy for a throwaway core. `kumikit --help`
lists the other commands (`plugin`, `pair`, `devices`, `bridge`).

### 3. The UI bundle

```sh
node scripts/bundle.mjs
```

This builds the UI kernel (desktop, browser and test builds), the first-party UI plugins
and themes, and `provider-acp` (for `wasm32-wasip2`), and assembles them in
`target/bundle/`:

| Directory | What |
|---|---|
| `ui/` | The UI kernel for the desktop app |
| `ui-browser/` | The browser client, with its UI plugins |
| `ui-browser-test/` | The build the Playwright tests use |
| `ui-plugins/` | The bundled UI plugins and themes |
| `core-plugins/` | `provider-acp` as a `.kkp`, preinstalled on a core started with `--bundled-plugins` |

Run it again after changing anything under `web/` or `plugins/`.

### 4. The desktop app

Build the core and the shell, then start the shell with the bundle's directories (an
installed app finds them next to itself):

```sh
cargo build -p kumikit-core -p kumikit-shell
KUMIKIT_UI_DIR="$PWD/target/bundle/ui" \
KUMIKIT_BUNDLED_UI_PLUGINS="$PWD/target/bundle/ui-plugins" \
KUMIKIT_BUNDLED_CORE_PLUGINS="$PWD/target/bundle/core-plugins" \
KUMIKIT_CORE_BIN="$PWD/target/debug/kumikit" \
  cargo run -p kumikit-shell
```

On Windows the core binary is `target/debug/kumikit.exe`. The app connects to a running
local core or starts this one. To keep a development setup apart from your real data, also
set `KUMIKIT_SOCKET`, `KUMIKIT_CORE_DATA_DIR` and `KUMIKIT_CLIENT_DATA_DIR`
([06-client](spec/06-client.md#shell-rust-kumikit-shell) lists every override).
`--safe` starts it without UI plugins.

### 5. A remote core and the browser client

On the machine that should accept remote clients:

```sh
kumikit serve --listen 0.0.0.0              # or one interface: --listen 192.168.1.5
kumikit pair                                # a one-time code and the certificate fingerprint
```

In the desktop app, add the core from **Cores** with its address and the code, and check
that the fingerprint the app shows matches the one `kumikit pair` printed. Remote access is
off unless `--listen` is given ([02-protocol](spec/02-protocol.md#remote-transport)).

To use a plain browser instead, also serve the browser client, then open
`https://<host>:7463/` and pair from the page:

```sh
kumikit serve --listen 0.0.0.0 --web target/bundle/ui-browser
```

For UI work with hot reload, run Vite's dev server against such a core (development only;
it listens on `localhost`):

```sh
npm run dev:browser -- --core https://127.0.0.1:7463 --fingerprint "SHA256:…"
```

### 6. Installers

Installers bundle the release core, the UI bundle and the app. CI builds them on each
platform (`installers` job); locally:

```sh
node scripts/bundle.mjs
cargo build --release -p kumikit-core
triple=$(rustc -vV | sed -n 's/^host: //p')
ext=""; case "$(uname -s)" in MINGW*|MSYS*) ext=.exe ;; esac
mkdir -p crates/kumikit-shell/binaries
cp "target/release/kumikit$ext" "crates/kumikit-shell/binaries/kumikit-$triple$ext"
cd crates/kumikit-shell
npx -y @tauri-apps/cli@2.12.5 build --config tauri.bundle.conf.json --bundles deb,rpm   # or: appimage, dmg, nsis
```

The results are in `target/release/bundle/`. An AppImage is built on Ubuntu 22.04 (so it
runs on glibc 2.35 and newer) and then repacked with `node scripts/repack-appimage.mjs
<file>` ([10-distribution](spec/10-distribution.md#desktop-installers)). The installers
are unsigned.

## Tests

The full list of checks, with when each applies, is [AGENTS.md → Verify](../AGENTS.md#verify).
The ones that need more setup:

- **Rust tests:** `cargo test --workspace` (builds the test plugins for `wasm32-wasip2`).
- **Web:** `npm run check` and `npm test`.
- **Browser end-to-end (Playwright, pinned to 1.63.0):** once, `npx playwright install
  chromium webkit` in `web/e2e`; then `node scripts/bundle.mjs && cargo build -p
  kumikit-core -p kumikit-web-relay` and `npx playwright test` in `web/e2e`.
- **Desktop end-to-end:** Linux, in the container: `sh scripts/desktop/run.sh sh
  scripts/desktop/e2e.sh` after `node scripts/bundle.mjs`. Windows runs it in CI.
- **Footprint:** `sh scripts/footprint.sh` (release build; core budgets, Linux numbers are
  the gated ones).

## Troubleshooting

- **`cargo test` fails building the test plugins:** the `wasm32-wasip2` target is missing.
  Under rustup it comes with `rust-toolchain.toml`; otherwise `rustup target add
  wasm32-wasip2`, or install your distribution's Rust WASM package.
- **The relay or Playwright says to run `node scripts/bundle.mjs`:** `target/bundle/` is
  missing or stale.
- **A blank desktop window on Linux:** try `WEBKIT_DISABLE_DMABUF_RENDERER=1`, which the
  tests set under Xvfb.
- **Windows: `sh` not found:** run the command in Git Bash.
