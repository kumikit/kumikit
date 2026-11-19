#!/bin/sh
# Builds the component and packs package/ into a .kkp, signed if KUMIKIT_KEY names your
# private key file (make one with `kumikit plugin keygen`; never commit it).
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/my_provider.wasm package/plugin.wasm
id=$(sed -n 's/^id = "\(.*\)"/\1/p' package/plugin.toml)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' package/plugin.toml)
if [ -n "${KUMIKIT_KEY:-}" ]; then
  kumikit plugin pack package -o "$id-$version.kkp" --key "$KUMIKIT_KEY"
else
  kumikit plugin pack package -o "$id-$version.kkp"
fi
