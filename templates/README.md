# Plugin templates

Starting points for plugin authors, kept working by `scripts/check-templates.sh` (CI's
`templates` job builds, packs, signs and installs each one):

- [core-plugin-rust](core-plugin-rust/README.md): a provider, as a WASM component.
- [ui-plugin-svelte](ui-plugin-svelte/README.md): a sandboxed panel in Svelte.

Themes are plain `.json` files rather than plugins; see `themes/` and
[docs/spec/08-themes.md](../docs/spec/08-themes.md).
