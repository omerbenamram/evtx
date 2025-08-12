#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   scripts/run_benchmark_pair.sh <clean_binary> <mod_binary> [evtx_file]
# Defaults:
#   evtx_file = samples/security_big_sample.evtx
# Both binaries are invoked with: -o xml -t 1 <evtx_file>
# Results are saved under benchmarks/ with a timestamped filename.

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CLEAN_BIN="${1:-}"
MOD_BIN="${2:-}"
EVTX_FILE="${3:-$REPO_DIR/samples/security_big_sample.evtx}"

if [[ -z "${CLEAN_BIN}" || -z "${MOD_BIN}" ]]; then
  echo "Usage: $0 <clean_binary> <mod_binary> [evtx_file]" >&2
  exit 1
fi

if [[ ! -x "$CLEAN_BIN" ]]; then
  echo "Clean binary not found or not executable: $CLEAN_BIN" >&2
  exit 1
fi
if [[ ! -x "$MOD_BIN" ]]; then
  echo "Modified binary not found or not executable: $MOD_BIN" >&2
  exit 1
fi

mkdir -p "$REPO_DIR/benchmarks"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT_FILE="$REPO_DIR/benchmarks/benchmark_pair_${TS}.txt"

echo "Benchmarking pair:" | tee "$OUT_FILE"
echo "  CLEAN: $CLEAN_BIN" | tee -a "$OUT_FILE"
echo "  MOD  : $MOD_BIN" | tee -a "$OUT_FILE"
echo "  FILE : $EVTX_FILE" | tee -a "$OUT_FILE"

hyperfine -w 2 \
  "$CLEAN_BIN -o xml -t 1 $EVTX_FILE > /dev/null" \
  "$MOD_BIN   -o xml -t 1 $EVTX_FILE > /dev/null" \
  | tee -a "$OUT_FILE"

printf "Saved results to %s\n" "$OUT_FILE" | tee -a "$OUT_FILE"


