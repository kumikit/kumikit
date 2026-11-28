# 08 — Themes

A theme is a **single JSON resource file**: metadata and design-token values. It runs no code
and needs no manifest, `.kkp` package, or signing ([ADR 0037](../decisions/0037-themes-as-resource-files.md)).

## Format

```json
{
  "$schema": "https://kumikit.dev/schema/theme-0.1.json",
  "id": "tokyo-night",
  "name": "Tokyo Night",
  "appearance": "dark",
  "tokens": {
    "surface.frame": "#16161e",
    "surface.panel": "#1a1b26",
    "text.primary": "#c0caf5",
    "accent": "#7aa2f7",
    "radius.medium": "8px"
  }
}
```

- `name` (non-empty string), `appearance` (`"light"` or `"dark"`), and `tokens` (a JSON
  object of token names to values) are required.
- `id` (lowercase letters, digits, and dashes) is required for bundled themes and optional
  for user-imported files (derived from the file name or `name` when omitted).
- Checked against `sdk/schema/theme.schema.json`.

## Bundled and custom themes

- Bundled themes live in `themes/*.json` (the default `kumikit-light` and `kumikit-dark`
  themes plus a curated gallery of popular light and dark themes). `scripts/bundle.mjs`
  writes `themes/index.json` alongside the desktop and browser UI bundles, and the UI
  kernel loads them in one fetch (including in safe mode).
- Users manage themes in **Settings → Appearance**: every theme shows a preview thumbnail
  drawn with its resolved palette, and **Add theme…** imports any `.json` theme file from
  disk into client settings (`Settings.customThemes`), working identically in desktop and
  browser clients. Imported themes can be removed from Appearance.

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

- Missing tokens fall back to the bundled default theme for the same appearance
  (`kumikit-light` or `kumikit-dark`, named in `bundled.json`), so a theme can be as small
  as an accent color. Without a fallback theme, the base styles use system colors.
- The theme engine writes tokens through the CSSOM (one constructed stylesheet), which
  the CSP allows without inline styles. The appearance follows the system unless the user
  picks a theme.
- Tokens map to CSS custom properties: `surface.panel` → `--kk-surface-panel`.
- Values are validated by type (color, length, font list, duration, shadow). Anything that
  is not a valid value of its type is rejected, which also rules out CSS injection.

## Validation

The UI kernel checks a theme when it is imported and loaded; theme authors run the same
checks with `kumikit theme check <file.json | dir> [--strict] [--json]`:
- Required structure: `name`, `appearance` (`light` or `dark`), optional valid `id`, and a
  `tokens` object.
- Token values by type (above). An invalid value or unknown token is an error.
- Contrast: text tokens against the surfaces they are used on must meet WCAG AA (4.5:1 for
  body text, 3:1 for large text and UI components). Failures are warnings; importing
  anyway is allowed with a warning, and `--strict` makes them errors.

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
