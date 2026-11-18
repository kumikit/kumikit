# 0019 — Sandboxed panels: iframes, a CSP per panel, a narrow bridge

- **Status:** Proposed
- **Date:** 2026-11-18

## Context

[07-ui-plugins](../spec/07-ui-plugins.md) describes level-2 UI plugins: code in an
iframe that talks to the UI kernel through a postMessage bridge, with no network unless
declared, and calls limited to `requires.core` methods and read-only kernel methods.
Third-party UI plugins should use this level, so they need no trust prompt. Open points:
the exact sandbox, how a CSP applies to each panel on three webviews and in the browser
client, the bridge's handshake, which slots may hold a panel, and the read scope.

## Decision

- **Frame:** `<iframe sandbox="allow-scripts">` from the plugin's own URL
  (`kumikit-plugin://<id>/…`, `http://kumikit-plugin.localhost/<id>/…` on WebView2,
  `/ui-plugins/<id>/…` in the browser client). No `allow-same-origin`, so the panel has an
  opaque origin; no top navigation, popups, forms or modals. The app's CSP allows frames
  from the plugin scheme only.
- **CSP per panel:** responses from a panel plugin's directory carry their own CSP:
  scripts, styles, images and fonts only from that plugin's directory, `connect-src` only
  to the `https` hosts it declares in `[ui] network` (shown at install), no frames, forms
  or base changes.
- **Bridge:** at the frame's first load the UI kernel transfers one `MessagePort` to that
  window; everything else travels on the port, and `window` messages from other sources
  are ignored. If the frame loads another document, the UI kernel removes it and closes
  the port.
- **SDK:** `@kumikit/ui-sdk/panel` gives the level-3 API asynchronously
  (`connectPanel()`), and applies theme tokens pushed by the UI kernel through a
  constructed stylesheet. The SDK goes to 0.1.1 (additive).
- **Policy:** a panel calls only its tab's connection: read-only kernel methods
  (`workspace.list`, `thread.list`, `thread.get`, `provider.list`, `provider.models`,
  `plugin.list`, `thread:` and `workspace:` subscriptions), thread and workspace reads
  limited to the tab's workspace, and `<pluginId>/*` methods of its `requires.core`
  plugins. No shell commands (profiles, plugin store), no other connections, no writes.
- **Slots:** panels may contribute to `sidebar.section`, `inspector.tab`, `view` and
  `settings.page`.

## Consequences

- Third-party UI plugins can be installed without trust; inline (level 3) keeps the trust
  prompt, and an update that moves a plugin from iframe to inline asks again.
- Each open panel costs a document in the webview (a few MB).
- Panels cannot render transcript items or the thread view; those stay inline.
- Tauri's IPC must be unreachable from a panel on every webview; a test proves it on
  WebKitGTK and WebView2 through WebDriver and on WKWebView through a test-hooks report.
