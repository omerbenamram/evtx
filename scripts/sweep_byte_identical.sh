#!/usr/bin/env bash
# Byte-identical sweep: render every sample in every output mode with a given
# evtx_dump binary, emitting one md5 per (sample, mode). Compare the output of
# two runs (baseline vs candidate) to prove rendering is unchanged.
#
# Usage: sweep_byte_identical.sh <evtx_dump-binary> <out-dir>
set -u
BIN="$1"
OUT="$2"
mkdir -p "$OUT"
SUM="$OUT/sums.txt"
# md5sum on Linux, md5 on macOS.
MD5=$(command -v md5sum || command -v md5)
: > "$SUM"

# Single-threaded so output order is deterministic across runs.
modes=(
  "xml::-o xml -t 1"
  "xml_noindent::-o xml --no-indent -t 1"
  "json::-o json -t 1"
  "jsonl::-o jsonl -t 1"
  "json_sep::-o json -t 1 --separate-json-attributes"
)

for f in samples/*.evtx; do
  base=$(basename "$f")
  for m in "${modes[@]}"; do
    name="${m%%::*}"
    args="${m##*::}"
    # Capture stdout (rendered records) and md5 it; stderr (logs) is dropped.
    sum=$("$BIN" $args "$f" 2>/dev/null | "$MD5" | cut -d" " -f1)
    echo "$base | $name | $sum" >> "$SUM"
  done
done
sort -o "$SUM" "$SUM"
echo "wrote $(wc -l < "$SUM") sums to $SUM"
