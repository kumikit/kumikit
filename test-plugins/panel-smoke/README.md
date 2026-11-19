# panel-smoke

Checks sandboxed panels (level 2, [07](../../docs/spec/07-ui-plugins.md#level-2-sandboxed-panels))
in the real desktop shell on every webview, without WebDriver (macOS has none for
WKWebView). `scripts/desktop/panel-smoke.mjs` starts a `test-hooks` build of the shell with
`ui/` as its UI and `plugins/` as its bundled UI plugins. The page frames the panel, which
probes its sandbox and reports through the bridge's `MessagePort`; the page relays the
results to the shell's stdout (`test_report`), and the script checks them.
