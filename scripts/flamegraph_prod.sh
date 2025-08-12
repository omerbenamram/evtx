#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/flamegraph_prod.sh /path/to/binary
# Environment variables (defaults mirror Makefile):
#   OUT_DIR (default: profile)
#   INPUT (default: ./samples/security_big_sample.evtx)
#   RUN_ARGS (ignored here; we use the same fixed args as Makefile)
#   FEATURES (unused)
#   DURATION (default: 30)
#   FREQ (default: 997)
#   FORMAT (default: jsonl)
#   FLAME_FILE (default: $INPUT)
#   BINARY (unused; we take BIN via positional arg)
#   FLAMEGRAPH_REPO_URL (default: https://github.com/brendangregg/FlameGraph.git)
#   FLAMEGRAPH_DIR (default: scripts/FlameGraph)
#   NO_INDENT_ARGS (default: --no-indent --dont-show-record-number)

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 /path/to/binary" >&2
  exit 1
fi

BIN="$1"
OS="$(uname -s)"
OUT_DIR="${OUT_DIR:-profile}"
INPUT="${INPUT:-./samples/security_big_sample.evtx}"
DURATION="${DURATION:-30}"
FREQ="${FREQ:-997}"
FORMAT="${FORMAT:-jsonl}"
FLAME_FILE="${FLAME_FILE:-$INPUT}"
FLAMEGRAPH_REPO_URL="${FLAMEGRAPH_REPO_URL:-https://github.com/brendangregg/FlameGraph.git}"
FLAMEGRAPH_DIR="${FLAMEGRAPH_DIR:-scripts/FlameGraph}"
NO_INDENT_ARGS="${NO_INDENT_ARGS:---no-indent --dont-show-record-number}"

# install-flamegraph (verbatim)
mkdir -p scripts
if [ ! -f "${FLAMEGRAPH_DIR}/stackcollapse.pl" ]; then
  echo "Cloning FlameGraph scripts..."
  git clone "${FLAMEGRAPH_REPO_URL}" "${FLAMEGRAPH_DIR}" >/dev/null
else
  echo "FlameGraph already present"
fi

# Clean output directory robustly even if root-owned
sudo rm -rf "${OUT_DIR}" >/dev/null 2>&1 || true
mkdir -p "${OUT_DIR}"

# folded-prod (verbatim Linux branch)
if [ "${OS}" = "Darwin" ]; then
  # macOS: sample the running process and collapse
  ( "${BIN}" -t 1 -o "${FORMAT}" ${NO_INDENT_ARGS} "${FLAME_FILE}" >/dev/null 2>&1 & echo $! > "${OUT_DIR}/pid" )
  sample "$(cat "${OUT_DIR}/pid")" "${DURATION}" -mayDie | tee "${OUT_DIR}/sample.txt" >/dev/null 2>&1 || true
  if kill -0 "$(cat "${OUT_DIR}/pid")" >/dev/null 2>&1; then kill -INT "$(cat "${OUT_DIR}/pid")" >/dev/null 2>&1 || true; fi
  wait "$(cat "${OUT_DIR}/pid")" 2>/dev/null || true
  awk -f "${FLAMEGRAPH_DIR}/stackcollapse-sample.awk" "${OUT_DIR}/sample.txt" > "${OUT_DIR}/stacks.folded"
else
  # Linux: record with perf and collapse
  sudo perf record -F "${FREQ}" -g -- "${BIN}" -t 1 -o "${FORMAT}" ${NO_INDENT_ARGS} "${FLAME_FILE}" >/dev/null
  sudo perf script > "${OUT_DIR}/perf.script"
  COLLAPSE_BIN="$(which inferno-collapse-perf)"
  if [ -z "${COLLAPSE_BIN}" ]; then
    echo "inferno-collapse-perf not found in PATH" >&2
    exit 2
  fi
  sudo "${COLLAPSE_BIN}" < "${OUT_DIR}/perf.script" > "${OUT_DIR}/stacks.folded"
fi
echo "Collapsed stacks written to ${OUT_DIR}/stacks.folded"

# flamegraph-prod (verbatim)
"${FLAMEGRAPH_DIR}/flamegraph.pl" "${OUT_DIR}/stacks.folded" > "${OUT_DIR}/flamegraph.svg"
echo "Flamegraph written to ${OUT_DIR}/flamegraph.svg"
echo "Computing hotspot summaries (top_leaf, top_titles)..."
awk '{ \
  if (match($0, / ([0-9]+)$/)) { count=substr($0, RSTART+1, RLENGTH-1) } else { count=1 } \
  line=$0; sub(/ [0-9]+$/, "", line); \
  n=split(line, frames, ";"); \
  leaf=frames[n]; \
  leaf_counts[leaf]+=count; \
} END { \
  for (l in leaf_counts) printf "%12d %s\n", leaf_counts[l], l; \
}' "${OUT_DIR}/stacks.folded" | sort -nr > "${OUT_DIR}/top_leaf.txt"
perl -ne 'if (/<title>([^<]+) \((?:\d+(?:\.\d+)?\s+samples,\s+)?(\d+(?:\.\d+)?)%\)/) { print $2, " ", $1, "\n" }' "${OUT_DIR}/flamegraph.svg" | sort -nr | head -n 30 > "${OUT_DIR}/top_titles.txt"
echo "Top summaries written to ${OUT_DIR}/top_leaf.txt and ${OUT_DIR}/top_titles.txt"