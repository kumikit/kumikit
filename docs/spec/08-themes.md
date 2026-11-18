# 08 — Themes

A theme is a **data-only** UI plugin: design tokens and optional fonts. It runs no code, so
anyone can publish one and installing one carries no risk.

## Package

```toml
id = "community.tokyo-night"
name = "Tokyo Night"
version = "1.0.0"
kind = ["theme"]
api = "^0.1"
[theme]
variants = [
  { id = "dark", appearance = "dark", tokens = "dark.json" },
  { id = "storm", appearance = "dark", tokens = "storm.json" },
]
fonts = ["fonts/Inter.woff2"]   # optional, referenced from tokens
```

## Tokens

Tokens are grouped and versioned with the UI SDK: the token set of UI SDK 0.1 is
`web/sdk/api/0.1.0/tokens.json`, with each token's name, group and type
([ADR 0014](../decisions/0014-ui-sdk-versioning.md)). A theme supplies values; the UI kernel
and plugins use only the token names, as CSS custom properties.

| Group | Examples |
|---|---|
| Surface | `surface.frame`, `surface.panel`, `surface.card`, `surface.raised`, `surface.overlay` |
| Text | `text.primary`, `text.secondary`, `text.muted`, `text.on-accent` |
| Accent | `accent`, `accent.hover`, `accent.tint`, `focus` |
| Border | `border.subtle`, `border.strong` |
| State | `state.success`, `state.warning`, `state.danger`, `state.info` |
| Code | `code.background`, `syntax.keyword`, `syntax.string`, … |
| Type | `font.ui`, `font.mono`, `font.size.base`, `font.size.small` |
| Shape | `radius.small`, `radius.medium`, `radius.large` |
| Elevation | `shadow.card`, `shadow.raised`, `shadow.overlay` |
| Material | `overlay.blur`, `overlay.opacity` |
| Motion | `motion.duration.short`, `motion.ease.out` |

```json
{
  "$schema": "https://kumikit.dev/schema/tokens-0.1.json",
  "surface.panel": "#1a1b26",
  "text.primary": "#c0caf5",
  "accent": "#7aa2f7",
  "radius.medium": "8px"
}
```

- Missing tokens fall back to the fallback theme's values for the same appearance, so a
  theme can be as small as an accent color. The fallback theme is named in the app's
  `bundled.json`, not in code; without it, the base styles use system colors.
- The theme engine writes tokens through the CSSOM (one constructed stylesheet), which
  the CSP allows without inline styles. The appearance follows the system unless the user
  picks a variant.
- Tokens map to CSS custom properties: `surface.panel` → `--kk-surface-panel`.
- Values are validated by type (color, length, font list, duration, shadow). Anything that
  is not a valid value of its type is rejected, which also rules out CSS injection.

## Validation

The UI kernel checks a theme when it is installed and loaded; theme authors run the same
checks with `kumikit theme check <dir | file.kkp> [--strict] [--json]`:
- The manifest's structure ([07](07-ui-plugins.md#manifest)).
- Token values by type (above). An invalid value is an error.
- Contrast: text tokens against the surfaces they are used on must meet WCAG AA (4.5:1 for
  body text, 3:1 for large text and UI components). Failures are warnings; installing
  anyway is allowed with a warning, and `--strict` makes them errors.
- Every referenced font file exists in the package and is a web font (WOFF or WOFF2). A
  missing or other file is an error.

The two implementations (TypeScript in the UI kernel, Rust in the `kumikit-theme` crate
behind the CLI) share their data: the token set (`web/sdk/api/<version>/tokens.json`), the
contrast pairs (`web/sdk/theme-rules.json`) and a set of test vectors
(`web/sdk/tests/theme-vectors.json`) that both test suites must pass, so they cannot
disagree without a test failing.

## Platform modes

The theme engine adds rules on top of any theme, so themes cannot break them:
- Increase Contrast: borders become solid, shadows are dropped.
- Reduced transparency: overlays become opaque.
- Forced colors: system colors replace tokens.
- Reduced motion: motion tokens become zero.

## Importing VS Code themes

An optional plugin, `kumikit.theme-import-vscode`, converts a VS Code color theme into a
Kumikit theme package (colors only; nothing from the extension is executed). It is a plugin
so that the kernel does not depend on another editor's theme format.
