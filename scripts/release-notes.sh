#!/usr/bin/env bash
# Prints the release notes of version $1 (such as 1.2.3): the lines under its
# `## [1.2.3 - YYYY-MM-DD]` heading in CHANGELOG.md. Fails if there are none, so that no
# version is released without notes.
set -euo pipefail
notes="$(awk -v h="## [$1 " '/^## \[/ { found = index($0, h) == 1; next } found' CHANGELOG.md)"
if ! grep -q '[^[:space:]]' <<<"$notes"; then
  echo "CHANGELOG.md has no notes for $1" >&2
  exit 1
fi
printf '%s\n' "$notes"
