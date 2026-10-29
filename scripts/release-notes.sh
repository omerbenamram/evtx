#!/usr/bin/env bash
# Prints the release notes of version $1 (such as 1.2.3): the lines under its heading in
# CHANGELOG.md, `## [1.2.3] - YYYY-MM-DD` or `## [1.2.3 - YYYY-MM-DD]`. Fails if they hold
# nothing but blank lines and subheadings, so that no version is released without notes.
set -euo pipefail
notes="$(awk -v v="$1" '/^## \[/ {
  found = index($0, "## [" v "]") == 1 || index($0, "## [" v " ") == 1
  next
} found' CHANGELOG.md)"
if ! grep -q '^[[:space:]]*[^#[:space:]]' <<<"$notes"; then
  echo "CHANGELOG.md has no notes for $1" >&2
  exit 1
fi
printf '%s\n' "$notes"
