#!/bin/sh
# Builds, packs, signs and installs every plugin template (templates/), the way an author
# would, so the templates cannot rot (CI's `templates` job). Each template is copied into
# target/template-check/ (inside the repository, so the UI template finds the workspace's
# node_modules) and pointed at this checkout's SDKs. A publisher key is made for the run,
# outside the repository, and removed afterwards. Linux and macOS.
set -eu
root=$(pwd)
work="$root/target/template-check"
keys=$(mktemp -d)
core=$(mktemp -d)
cleanup() {
  [ -n "${serve_pid:-}" ] && kill "$serve_pid" 2>/dev/null || true
  rm -rf "$keys" "$core"
}
trap cleanup EXIT
rm -rf "$work"
mkdir -p "$work"
if [ "${KUMIKIT_SKIP_BUILD:-0}" != "1" ]; then
  cargo build -q -p kumikit-core -p kumikit-test-client-store
fi
kumikit="$root/target/debug/kumikit"
store="$root/target/debug/client-store"
"$kumikit" plugin keygen --out "$keys/template.key" >/dev/null
key="$keys/template.key"

echo "== core plugin (Rust)"
cp -R templates/core-plugin-rust "$work/core"
sed -i.bak 's|kumikit-provider = { git = "https://github.com/kumikit/kumikit" }|kumikit-provider = { path = "'"$root"'/sdk/rust-provider" }|' "$work/core/Cargo.toml"
grep -q 'path = ' "$work/core/Cargo.toml"
(cd "$work/core" && CARGO_TARGET_DIR="$root/target/template-check-target" cargo build -q --release --target wasm32-wasip2 \
  && CARGO_TARGET_DIR="$root/target/template-check-target" cargo clippy -q --release --target wasm32-wasip2 -- -D warnings)
cp "$root/target/template-check-target/wasm32-wasip2/release/my_provider.wasm" "$work/core/package/plugin.wasm"
"$kumikit" plugin pack "$work/core/package" -o "$work/echo.kkp" --key "$key"
"$kumikit" plugin verify "$work/echo.kkp"
chmod 700 "$core"
"$kumikit" plugin add "$work/echo.kkp" --yes --data-dir "$core/data" --socket "$core/s.sock" | tee "$work/add.txt"
grep -q "Signed by key" "$work/add.txt"
grep -q "Enabled yourname.echo" "$work/add.txt"
mkdir -p "$core/proj"
"$kumikit" serve --data-dir "$core/data" --socket "$core/s.sock" > "$work/serve.txt" &
serve_pid=$!
i=0; until grep -q '^ready ' "$work/serve.txt"; do i=$((i+1)); [ $i -lt 300 ] || { echo "the core did not start"; exit 1; }; sleep 0.1; done
node scripts/template-turn.mjs "$core/s.sock" "$core/proj" yourname.echo/echo "hello template" "hello template"
kill "$serve_pid"; serve_pid=

echo "== UI plugin (Svelte)"
cp -R templates/ui-plugin-svelte "$work/ui"
(cd "$work/ui" && KUMIKIT_UI_SDK="$root/web/sdk" npx vite build --logLevel warn)
"$kumikit" plugin pack "$work/ui/package" -o "$work/panel.kkp" --key "$key"
"$store" "$work/client/ui-plugins" "$work/panel.kkp" | tee "$work/ui.txt"
grep -q "installed yourname.panel 0.1.0 signed" "$work/ui.txt"

echo "== bundled themes"
"$kumikit" theme check themes --strict
echo "templates ok"
