#!/bin/sh
# Fails if a tracked file holds a PEM private key (10-distribution, Publisher keys):
# publisher keys belong in the user's config directory, never in the repository. Tests
# generate their keys at run time. A PEM block starts a line, so code that only names the
# header (in quotes or backticks) does not match.
set -eu
found=$(git ls-files -z | xargs -0 grep -lE '^-----BEGIN ([A-Z]+ )?PRIVATE KEY-----' 2>/dev/null || true)
if [ -n "$found" ]; then
  echo "private keys are committed:" >&2
  echo "$found" >&2
  exit 1
fi
echo "no private keys"
