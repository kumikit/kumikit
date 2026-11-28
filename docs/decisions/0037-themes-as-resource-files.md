# 0037 — Themes as plain JSON resource files

- **Status:** Accepted
- **Date:** 2026-11-28

## Context

Themes were previously packaged, signed, and installed as client plugins (`kind = ["theme"]`, `.kkp` archives with `plugin.toml` and per-variant JSON files) through the client plugin store and registry. That machinery exists to keep executable code safe, yet themes run no code at all — they only supply validated design-token values (mostly a color palette).

Requiring authors to generate signing keys, write a manifest, pack a `.kkp`, and publish to a registry made sharing or tweaking a color palette needlessly heavy, and cluttering the Plugins manager with themes split theme selection across two places (Plugins and Settings → Appearance).

## Decision

1. **Themes are single `.json` resource files, not plugins.**
   - A theme file is a JSON object containing `name`, `appearance` (`"light"` or `"dark"`), optional `id` and `$schema`, and a `tokens` map of UI SDK 0.1 token names to values (`sdk/schema/theme.schema.json`).
   - Missing tokens fall back to the bundled default theme for the same `appearance` (`kumikit-light` or `kumikit-dark`), so a theme can override as little as an accent color or a core palette.
   - Token validation (`checkTokens`) and WCAG AA contrast checks (`contrastWarnings`) run unchanged on load and import, keeping CSS injection impossible and warning on low contrast.
2. **Bundled themes ship as static resources.**
   - `themes/*.json` at the repository root holds the bundled defaults (`kumikit-light.json`, `kumikit-dark.json`) plus a curated gallery of popular developer themes (Tokyo Night, Dracula, Monokai, Catppuccin, Rosé Pine, Gruvbox, Solarized, Nord, One Dark, GitHub, Everforest, Kanagawa, Ayu, Material Ocean, Night Owl, Palenight, etc.).
   - `scripts/bundle.mjs` indexes them into `themes/index.json` alongside the UI kernel for both the desktop app and the browser client, loaded in a single fetch (including in safe mode).
3. **User themes are imported directly in Settings → Appearance.**
   - Appearance displays the bundled themes with live SVG preview thumbnails (using SVG presentation attributes, which require no `unsafe-inline` CSS under the strict CSP) and an "Add theme…" action that picks a `.json` file from disk.
   - Imported themes are validated and stored in client settings (`Settings.customThemes`), working identically in desktop and browser with no new shell commands ([ADR 0013](0013-desktop-shell.md)). Users can remove imported themes from Appearance.
4. **`kind = ["theme"]` plugin support is removed.**
   - UI plugin manifests (`kumikit-assets`, `kumikit-shell`, `kumikit-registry`, `kumikit-registry-tool`, `sdk/schema/ui-plugin-manifest.schema.json`) accept only `kind = ["ui"]`.
   - `plugins/theme-default` and `templates/theme` are removed.
   - `kumikit theme check <file.json | dir> [--strict] [--json]` checks a single `.json` theme file or every `.json` theme file in a directory.
5. **UI SDK 0.1.7** adds optional `source` and `preview` fields on `ThemeVariantInfo` and optional `importFromFile?()` / `remove?()` methods on `PluginContext['theme']`, leaving `web/sdk/api/0.1.0/` untouched ([ADR 0014](0014-ui-sdk-versioning.md)).

## Consequences

- Adding, previewing, switching, and sharing themes requires zero packaging, signing, or registry setup.
- Both desktop and browser clients get the exact same bundled theme gallery and local `.json` import flow.
- Existing `.kkp` packages with `kind = ["theme"]` are no longer accepted by the client plugin store or registry checker.
