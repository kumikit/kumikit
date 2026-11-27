# Model picker, model settings and provider icons

- **Goal:** one button under every composer opens a rich model picker (search, recent
  models, providers with icons, models, and per-model settings such as effort and speed);
  providers describe their settings and ship icons
  ([06-client](../spec/06-client.md), [05-providers](../spec/05-providers.md),
  [ADR 0033](../decisions/0033-model-settings-and-provider-icons.md)).
- **Status:** done

## Steps

- [x] WIT `kumikit:models@0.1.0` (`interface options`, `set-options` during `list-models`),
  linked for every provider; `kumikit-provider` SDK module `models`; kernel `Model.options`.
  Verified by a plugin-host test with a test provider that reports options.
- [x] Protocol 0.11: `ModelInfo.options`, `ModelOption`, `ModelOptionValue`,
  `ProviderInfo.icon`, `Thread.options`, `options` on `thread.create` / `thread.update`;
  fixtures and generated TypeScript; `template-turn.mjs` protocol version.
- [x] Kernel: migration `0018_thread_options.sql`; validate options against the model;
  drop invalid ones on a model change; forks and side threads inherit; options reach the
  session as `option.<id>`; a change restarts the live session. Kernel tests.
- [x] Manifest `icon` on provider entries (schema, strict type, package check: SVG or PNG,
  at most 32 KiB, inside the package); host loads it into a `data:` URL;
  `Provider::icon`; `provider.list` returns it. Tests per rule.
- [x] `provider-acp`: every ACP `configOptions` entry of type `select` other than `mode`
  and `model` becomes a model option (`thought_level` → kind `effort`); chosen values go
  out with `session/set_config_option` after `session/new`. Replay tests.
- [x] UI SDK 0.1.6: `ctx.ui.pickModel(anchor, options)`; kernel `ModelPicker.svelte`
  (search, Recent, providers with icons or a letter badge, models, option rows);
  recent picks per connection in client storage; used by `NewThreadComposer` and
  `ui-chat`. Vitest + Playwright.
- [x] `kumikit-plugin-agents`: `provider-claude` reports `effort` (`--effort`) and `speed`
  (`--settings {"fastMode":true}`) on its models; icons for every provider, with their
  source noted. Its own tests.
- [x] Spec: 02-protocol, 04-plugin-host, 05-providers, 06-client, 07-ui-plugins,
  10-distribution; roadmap note.

## Notes

- Session events (`kumikit:session@0.1.0`) set the pattern: an additive import linked for
  every provider, not listed in `runtime.worlds`.
- Claude Code over ACP advertises `effort` (`thought_level`) but no speed; native
  `provider-claude` can offer fast mode through `--settings`.
