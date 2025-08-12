#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "Usage: $0 <folded.stacks> <out.svg> <out_top_leaf.txt> <out_top_titles.txt>" >&2
  exit 1
fi

FOLDED="$1"
SVG_OUT="$2"
LEAF_OUT="$3"
TITLES_OUT="$4"

# Generate SVG
inferno-flamegraph < "$FOLDED" > "$SVG_OUT"

# Top leaves by sample count
awk '{ \
  if (match($0, / ([0-9]+)$/)) { count=substr($0, RSTART+1, RLENGTH-1) } else { count=1 } \
  line=$0; sub(/ [0-9]+$/, "", line); \
  n=split(line, frames, ";"); \
  leaf=frames[n]; \
  leaf_counts[leaf]+=count; \
} END { \
  for (l in leaf_counts) printf "%12d %s\n", leaf_counts[l], l; \
}' "$FOLDED" | sort -nr | head -n 30 > "$LEAF_OUT"

# Top titles with percent parsed from SVG
perl -ne 'if (/<title>([^<]+) \((?:\d+(?:\.\d+)?\s+samples,\s+)?(\d+(?:\.\d+)?)%\)/) { print $2, " ", $1, "\n" }' "$SVG_OUT" | sort -nr | head -n 30 > "$TITLES_OUT"