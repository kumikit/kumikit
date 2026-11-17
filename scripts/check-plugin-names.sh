#!/bin/sh
# The kernel knows no plugin by name (AGENTS.md, architecture rules; ADR 0006): fails if
# the code in crates/*/src, web/kernel/src or web/sdk/src names a plugin id, a qualified
# provider id or a provider backend key from any plugin.toml under plugins/ or
# test-plugins/.
# Tests (crates/*/tests, and unit-test modules in tests.rs files) may name plugins;
# they are not kernel code.
# Usage: sh scripts/check-plugin-names.sh   (run from the repository root)
set -eu

manifests=$(find plugins test-plugins -name plugin.toml | sort)
[ -n "$manifests" ] || { echo "no plugin.toml found" >&2; exit 1; }

terms=$(mktemp)
trap 'rm -f "$terms"' EXIT
for m in $manifests; do
    id=$(sed -n 's/^id *= *"\([^"]*\)".*/\1/p' "$m" | head -n 1)
    [ -n "$id" ] || { echo "$m: no id" >&2; exit 1; }
    echo "$id" >> "$terms"
    # Provider entries: { id = "local", ... } -> <plugin id>/<local id>
    grep -o '{ *id *= *"[^"]*"' "$m" | sed 's/.*"\([^"]*\)"/\1/' | while read -r local; do
        echo "$id/$local" >> "$terms"
    done
    # Backend keys, as string literals: key = "acp" -> "acp"
    grep -o 'key *= *"[^"]*"' "$m" | sed 's/.*\("[^"]*"\)/\1/' >> "$terms"
done

status=0
while read -r term; do
    if hits=$(grep -rnF --exclude=tests.rs -- "$term" crates/*/src web/kernel/src web/sdk/src); then
        echo "kernel code names a plugin ($term):" >&2
        echo "$hits" >&2
        status=1
    fi
done < "$terms"
[ "$status" -eq 0 ] && echo "plugin names ok ($(wc -l < "$terms") names checked)"
exit "$status"
