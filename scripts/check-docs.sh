#!/bin/sh
# Documentation checks: AGENTS.md stays small, and relative Markdown links resolve.
# Usage: sh scripts/check-docs.sh   (run from the repository root)
set -eu

LIMIT=32768
status=0

size=$(wc -c < AGENTS.md | tr -d ' ')
if [ "$size" -ge "$LIMIT" ]; then
  echo "AGENTS.md is $size bytes (limit $LIMIT); move detail into docs/spec/." >&2
  status=1
fi

# Every relative link target in tracked or new Markdown files must exist.
# Anchors (#...) are stripped; external links and mailto are skipped.
files=$(git ls-files --cached --others --exclude-standard '*.md')
for file in $files; do
  dir=$(dirname "$file")
  # Pull out "](target)" occurrences, one per line, outside fenced code blocks.
  awk '/^```/ { fence = !fence; next } !fence' "$file" |
    grep -o '](\([^)]*\))' | sed 's/^](\(.*\))$/\1/' |
    while IFS= read -r target; do
      case "$target" in
        http://*|https://*|mailto:*|\#*|'') continue ;;
      esac
      path=${target%%#*}
      [ -n "$path" ] || continue
      if [ ! -e "$dir/$path" ]; then
        echo "$file: broken link -> $target" >&2
        echo broken
      fi
    done
done > /tmp/kumikit-check-docs.$$ || true

if [ -s /tmp/kumikit-check-docs.$$ ]; then
  status=1
fi
rm -f /tmp/kumikit-check-docs.$$

if [ "$status" -eq 0 ]; then
  echo "docs ok"
fi
exit "$status"
