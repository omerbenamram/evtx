#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZIG_ROOT="${ZIG_ROOT:-/Users/omerba/Workspace/zig-evtx}"

OUT_DIR="${OUT_DIR:-$ROOT/perf}"
DATA_PATH="${DATA_PATH:-$OUT_DIR/utf16_escape_data.bin}"
RUST_OUT="${RUST_OUT:-$OUT_DIR/utf16_escape_rust.csv}"
ZIG_OUT="${ZIG_OUT:-$OUT_DIR/utf16_escape_zig.csv}"
MATRIX_OUT="${MATRIX_OUT:-$OUT_DIR/utf16_escape_matrix.md}"

mkdir -p "$OUT_DIR"

python3 "$ROOT/scripts/gen_utf16_escape_dataset.py" --out "$DATA_PATH"

(cd "$ROOT" && cargo build --release --bin bench_utf16_escape_matrix)
(cd "$ZIG_ROOT" && zig build -Doptimize=ReleaseFast)

"$ROOT/target/release/bench_utf16_escape_matrix" --data "$DATA_PATH" > "$RUST_OUT"
"$ZIG_ROOT/zig-out/bin/bench_utf16_escape_matrix" --data "$DATA_PATH" > "$ZIG_OUT"

python3 "$ROOT/scripts/merge_utf16_escape_matrix.py" \
  --rust "$RUST_OUT" \
  --zig "$ZIG_OUT" \
  --out "$MATRIX_OUT"

echo "Rust CSV: $RUST_OUT"
echo "Zig  CSV: $ZIG_OUT"
echo "Matrix : $MATRIX_OUT"
