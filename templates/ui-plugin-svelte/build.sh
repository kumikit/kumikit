#!/bin/sh
# Builds the panel into package/dist and packs package/ into a .kkp, signed if KUMIKIT_KEY
# names your private key file. Needs KUMIKIT_UI_SDK (see vite.config.ts).
set -eu
cd "$(dirname "$0")"
npx vite build --logLevel warn
id=$(sed -n 's/^id = "\(.*\)"/\1/p' package/plugin.toml)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' package/plugin.toml)
if [ -n "${KUMIKIT_KEY:-}" ]; then
  kumikit plugin pack package -o "$id-$version.kkp" --key "$KUMIKIT_KEY"
else
  kumikit plugin pack package -o "$id-$version.kkp"
fi
