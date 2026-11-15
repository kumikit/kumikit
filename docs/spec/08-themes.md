# 08 — Themes

A theme is a **data-only** UI plugin: design tokens and optional fonts. It runs no code, so
anyone can publish one and installing one carries no risk.

## Package

```toml
id = "community.tokyo-night"
name = "Tokyo Night"
version = "1.0.0"
kind = ["theme"]
api = "^1.0"
[theme]
variants = [
  { id = "dark", appearance = "dark", tokens = "dark.json" },
  { id = "storm", appearance = "dark", tokens = "storm.json" },
]
fonts = ["fonts/Inter.woff2"]   # optional, referenced from tokens
```

## Tokens

Tokens are grouped and versioned with the UI SDK (`tokens@1`). A theme supplies values; the
UI kernel and plugins use only the token names, as CSS custom properties.

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
  "$schema": "https://kumikit.dev/schema/tokens-1.json",
  "surface.panel": "#1a1b26",
  "text.primary": "#c0caf5",
  "accent": "#7aa2f7",
  "radius.medium": "8px"
}
```

- Missing tokens fall back to the default theme's values for the same appearance, so a
  theme can be as small as an accent color.
- Tokens map to CSS custom properties: `surface.panel` → `--kk-surface-panel`.
- Values are validated by type (color, length, font list, duration, shadow). Anything that
  is not a valid value of its type is rejected, which also rules out CSS injection.

## Validation

At install and in the theme author's tooling (`kumikit theme check`):
- Contrast: text tokens against the surfaces they are used on must meet WCAG AA (4.5:1 for
  body text, 3:1 for large text and UI components). Failures are reported; installing
  anyway is allowed with a warning.
- Every referenced font file exists and is a web font.

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
