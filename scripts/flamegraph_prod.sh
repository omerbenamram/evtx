#!/usr/bin/env bash
set -euo pipefail

# Simple production-style flamegraph helper using perf + inferno (Linux)
# or cargo-flamegraph (macOS).
# Intended to be invoked via `make flamegraph-prod` with environment
# overrides, e.g.:
#   FLAME_FILE=samples/security_big_sample.evtx \
#   FORMAT=json \
#   DURATION=30 \
#   BIN=./target/release/evtx_dump \
#   make flamegraph-prod
#
OS="$(uname -s || echo unknown)"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Optional label for this run (used in output filenames).
: "${TAG:=default}"

: "${BIN:=$ROOT_DIR/target/release/evtx_dump}"
: "${FLAME_FILE:=$ROOT_DIR/samples/security_big_sample.evtx}"
: "${FORMAT:=json}"
: "${DURATION:=30}"
# For JSON formats, choose parser implementation: streaming | legacy.
: "${JSON_PARSER:=streaming}"
: "${OUT_DIR:=$ROOT_DIR/profile}"

mkdir -p "$OUT_DIR"

echo "Profiling"
echo "  FLAME_FILE=$FLAME_FILE"
echo "  FORMAT=$FORMAT"
echo "  DURATION=${DURATION}s"
echo "  OUT_DIR=$OUT_DIR"
echo "  TAG=$TAG"

# Map FORMAT to evtx_dump arguments.
case "$FORMAT" in
  json|jsonl)
    # Use streaming JSON path by default; caller can change via JSON_PARSER env.
    FMT_ARGS=(-t 1 -o "$FORMAT" --json-parser "$JSON_PARSER")
    ;;
  xml)
    FMT_ARGS=(-t 1 -o xml)
    ;;
  *)
    echo "warning: unknown FORMAT='$FORMAT', defaulting to json" >&2
    FMT_ARGS=(-t 1 -o json --json-parser streaming)
    ;;
esac

if [[ "$OS" == "Darwin" ]]; then
  # macOS path: use cargo-flamegraph (wraps dtrace + inferno).
  if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found in PATH; required for cargo-flamegraph on macOS." >&2
    exit 1
  fi

  echo "Detected macOS; using cargo flamegraph (you may be prompted for sudo)."

  FOLDED_STACKS="$OUT_DIR/stacks_${TAG}.folded"

  # Ask cargo-flamegraph to tee the folded stacks into our own file.
  (cd "$ROOT_DIR" && \
    cargo flamegraph \
      --root \
      --bin evtx_dump \
      --output "$OUT_DIR/flamegraph_${TAG}.svg" \
      --post-process "tee $FOLDED_STACKS" \
      -- "${FMT_ARGS[@]}" "$FLAME_FILE")

  if [[ -f "$FOLDED_STACKS" ]] && [[ -s "$FOLDED_STACKS" ]]; then
    # Extract top leafs (leaf functions) from folded stacks
    {
      echo "Top leaf functions (by total samples):"
      awk '{
        n = split($1, stack, ";");
        if (n > 0) {
          leaf = stack[n];
          count = $2 + 0;
          leafs[leaf] += count;
        }
      }
      END {
        for (f in leafs) {
          printf "%d %s\n", leafs[f], f;
        }
      }' "$FOLDED_STACKS" | sort -nr | head -20 | awk '{printf "  %s: %s\n", $2, $1}'
    } > "$OUT_DIR/top_leaf_${TAG}.txt"

    # Extract top titles (root functions) from folded stacks
    {
      echo "Top title functions (by total samples):"
      awk '{
        n = split($1, stack, ";");
        if (n > 0) {
          title = stack[1];
          count = $2 + 0;
          titles[title] += count;
        }
      }
      END {
        for (f in titles) {
          printf "%d %s\n", titles[f], f;
        }
      }' "$FOLDED_STACKS" | sort -nr | head -20 | awk '{printf "  %s: %s\n", $2, $1}'
    } > "$OUT_DIR/top_titles_${TAG}.txt"

    echo "Top leafs written to $OUT_DIR/top_leaf_${TAG}.txt"
    echo "Top titles written to $OUT_DIR/top_titles_${TAG}.txt"
  else
    echo "warning: folded stacks file is empty or missing, skipping text summaries" >&2
  fi

  echo "Flamegraph written to $OUT_DIR/flamegraph_${TAG}.svg"
  exit 0
fi

# Linux / perf + inferno path.
#
# Requirements:
#   - perf
#   - inferno-collapse-perf
#   - inferno-flamegraph

if ! command -v perf >/dev/null 2>&1; then
  echo "error: perf not found in PATH; flamegraph_prod.sh currently expects Linux + perf." >&2
  exit 1
fi

if ! command -v inferno-collapse-perf >/dev/null 2>&1; then
  echo "error: inferno-collapse-perf not found in PATH." >&2
  exit 1
fi

if ! command -v inferno-flamegraph >/dev/null 2>&1; then
  echo "error: inferno-flamegraph not found in PATH." >&2
  exit 1
fi

perf record -F 999 -g --output "$OUT_DIR/perf.data" -- \
  "$BIN" "${FMT_ARGS[@]}" "$FLAME_FILE" >/dev/null

perf script -i "$OUT_DIR/perf.data" | inferno-collapse-perf > "$OUT_DIR/stacks.folded"
cat "$OUT_DIR/stacks.folded" | inferno-flamegraph > "$OUT_DIR/flamegraph_${TAG}.svg"

# Extract top leafs (functions at end of stack) and top titles (functions at start of stack)
# Folded format: "func1;func2;func3 12345" where number is sample count
{
  echo "Top leaf functions (by total samples):"
  awk '{
    n = split($1, stack, ";");
    if (n > 0) {
      leaf = stack[n];
      count = $2 + 0;
      leafs[leaf] += count;
    }
  }
  END {
    for (f in leafs) {
      printf "%d %s\n", leafs[f], f;
    }
  }' "$OUT_DIR/stacks.folded" | sort -nr | head -20 | awk '{printf "  %s: %s\n", $2, $1}'
} > "$OUT_DIR/top_leaf_${TAG}.txt"

{
  echo "Top title functions (by total samples):"
  awk '{
    n = split($1, stack, ";");
    if (n > 0) {
      title = stack[1];
      count = $2 + 0;
      titles[title] += count;
    }
  }
  END {
    for (f in titles) {
      printf "%d %s\n", titles[f], f;
    }
  }' "$OUT_DIR/stacks.folded" | sort -nr | head -20 | awk '{printf "  %s: %s\n", $2, $1}'
} > "$OUT_DIR/top_titles_${TAG}.txt"

echo "Flamegraph written to $OUT_DIR/flamegraph_${TAG}.svg"
echo "Top leafs written to $OUT_DIR/top_leaf_${TAG}.txt"
echo "Top titles written to $OUT_DIR/top_titles_${TAG}.txt"


