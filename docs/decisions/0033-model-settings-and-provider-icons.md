# 0033 — Model settings and provider icons

- **Status:** Accepted
- **Date:** 2026-11-27

## Context

Users want to choose more than a model: how hard the agent thinks (effort) and, where
the agent supports it, a faster mode. Agents expose these differently. Claude Code over
ACP lists an `effort` config option (category `thought_level`) in `session/new`; the
Claude Code CLI takes `--effort <level>` and a `fastMode` setting; other agents offer
nothing or something else. The kernel must not know any of these names
([ADR 0006](0006-kernel-owns-domain-model.md)).

The picker should also show each harness's icon, which only the plugin that declares the
provider can supply.

`kumikit:provider@0.2.0` is published: its `model` record cannot gain a field, and a
second export would need a new provider world that every caller handles separately.

## Decision

- **`kumikit:models@0.1.0`**, an additive package like `kumikit:session@0.1.0`, linked for
  every provider and not listed in `runtime.worlds`. Its `options` interface has
  `set-options(model, options)`, valid only while the host runs the provider's
  `list-models`: it attaches settings to a model that call returns. A setting is an id, a
  label, an optional `kind` hint (`effort` and `speed` are documented; others are free),
  its values (value, label, optional description) and an optional default. Outside
  `list-models` the call fails with `invalid-argument`.
- **Protocol 0.11** (minor): `ModelInfo.options`, `ProviderInfo.icon`, `Thread.options`,
  and `options` on `thread.create` and `thread.update`. The kernel stores the chosen
  values with the thread, checks them against the model's settings, drops values the new
  model does not offer when the model changes, and passes them to `start-session` as
  `option.<id>` entries of `session-config.options`. Changing them restarts the live
  session at the next turn, like a model change.
- **Manifest `icon`** on a `contributes.providers` entry: a path inside the package to an
  SVG or PNG of at most 32 KiB. The host reads it when it registers the provider and
  `provider.list` returns it as a `data:` URL. Clients show it with `<img>`, so an SVG
  cannot run scripts; the client CSP already allows `data:` images.
- **One picker, owned by the UI kernel**, offered to UI plugins as `ctx.ui.pickModel` in
  UI SDK 0.1.6, so the new-thread composer and `ui-chat` share it. Recent picks are kept
  per connection on the client.
- First-party provider plugins may ship their vendor's mark as the icon, to identify the
  harness; each icon's source is noted next to it and the marks stay their owners'.

## Consequences

- Providers built before this keep working and simply offer no settings or icon.
- Settings are per model, so a model without effort shows no Effort row.
- The kernel validates values but never interprets them; a provider that ignores an
  `option.<id>` it advertised is a provider bug.
- Changing a setting mid-thread costs a session restart (with `resume`), the same as
  changing the model today.
