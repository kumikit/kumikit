# A Kumikit theme

Design tokens for a dark and a light variant. A theme runs no code, so installing one is
safe; tokens it leaves out come from the default theme. Copy this folder to start your own.

## Make it yours

1. Set `id`, name, description and repository in `plugin.toml`.
2. Change the values in `dark.json` and `light.json`. The token names and their types are
   in `web/sdk/api/0.1.0/tokens.json` of the Kumikit repository
   ([08-themes](../../docs/spec/08-themes.md)). Fonts go in `fonts/` as WOFF2 and are
   listed in `[theme] fonts`.
3. Check it as the app will at install, contrast included:

   ```sh
   kumikit theme check . --strict
   ```

## Pack and sign

```sh
kumikit plugin pack . -o yourname.dusk-0.1.0.kkp --key ~/.config/kumikit/keys/yourname.key
```

Pack a folder that holds only the theme's files (this README is packed too, which is
fine). Install the `.kkp` in the app from Manage plugins → On this client.
