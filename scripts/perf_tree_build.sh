#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

FILE="$ROOT/samples/security.evtx"
LOOPS=500000
PROFILE_LOOPS=500000
WEIGHT="cpu"
OUT_DIR="$ROOT/perf"
SKIP_BUILD=0
SKIP_SAMPLY=0
SKIP_HYPERFINE=0

ZIG_ROOT="${ZIG_ROOT:-/Users/omerba/Workspace/zig-evtx}"
ZIG_BIN="${ZIG_BIN:-$ZIG_ROOT/zig-out/bin/bench_tree_build_loop}"

usage() {
  cat <<'EOF'
Usage: scripts/perf_tree_build.sh [options]

Options:
  --file <path>          EVTX input file (default: samples/security.evtx)
  --loops <n>            Loop count for hyperfine (default: 500000)
  --profile-loops <n>    Loop count for samply profiles (default: 500000)
  --weight <cpu|wall|samples>  Weight mode for tables (default: cpu)
  --out-dir <dir>        Output directory (default: perf)
  --skip-build           Skip cargo/zig builds
  --skip-hyperfine       Skip hyperfine run
  --skip-samply          Skip samply profiles + table extraction
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --file) FILE="$2"; shift 2;;
    --loops) LOOPS="$2"; shift 2;;
    --profile-loops) PROFILE_LOOPS="$2"; shift 2;;
    --weight) WEIGHT="$2"; shift 2;;
    --out-dir) OUT_DIR="$2"; shift 2;;
    --skip-build) SKIP_BUILD=1; shift;;
    --skip-hyperfine) SKIP_HYPERFINE=1; shift;;
    --skip-samply) SKIP_SAMPLY=1; shift;;
    -h|--help) usage; exit 0;;
    *) echo "Unknown arg: $1"; usage; exit 1;;
  esac
done

RUST_BIN="$ROOT/target/release/bench_tree_build"
RUST_BIN_DIRECT="$ROOT/target/release/bench_tree_build_direct"

mkdir -p "$OUT_DIR"

if [[ $SKIP_BUILD -eq 0 ]]; then
  (cd "$ROOT" && cargo build --release --features bench)
  (cd "$ZIG_ROOT" && zig build -Doptimize=ReleaseFast)
fi

if [[ $SKIP_HYPERFINE -eq 0 ]]; then
  hyperfine -w 3 -r 8 \
    "$RUST_BIN --file $FILE --loops $LOOPS" \
    "$RUST_BIN_DIRECT --file $FILE --loops $LOOPS" \
    "$ZIG_BIN --file $FILE --loops $LOOPS" \
    --export-json "$OUT_DIR/hyperfine_tree_build.json" \
    | tee "$OUT_DIR/hyperfine_tree_build.txt"
fi

if [[ $SKIP_SAMPLY -eq 0 ]]; then
  samply record --save-only --unstable-presymbolicate \
    -o "$OUT_DIR/rust_tree_build_loop.json.gz" -- \
    "$RUST_BIN" --file "$FILE" --loops "$PROFILE_LOOPS" > /dev/null

  samply record --save-only --unstable-presymbolicate \
    -o "$OUT_DIR/rust_tree_build_direct_loop.json.gz" -- \
    "$RUST_BIN_DIRECT" --file "$FILE" --loops "$PROFILE_LOOPS" > /dev/null

  samply record --save-only --unstable-presymbolicate \
    -o "$OUT_DIR/zig_tree_build_loop.json.gz" -- \
    "$ZIG_BIN" --file "$FILE" --loops "$PROFILE_LOOPS" > /dev/null

  python3 "$ROOT/scripts/samply_extract_tables.py" \
    --profile "$OUT_DIR/rust_tree_build_loop.json.gz" \
    --syms "$OUT_DIR/rust_tree_build_loop.json.syms.json" \
    --out-dir "$OUT_DIR" \
    --label rust_tree_build_loop \
    --weight "$WEIGHT"

  python3 "$ROOT/scripts/samply_extract_tables.py" \
    --profile "$OUT_DIR/rust_tree_build_direct_loop.json.gz" \
    --syms "$OUT_DIR/rust_tree_build_direct_loop.json.syms.json" \
    --out-dir "$OUT_DIR" \
    --label rust_tree_build_direct_loop \
    --weight "$WEIGHT"

  python3 "$ROOT/scripts/samply_extract_tables.py" \
    --profile "$OUT_DIR/zig_tree_build_loop.json.gz" \
    --syms "$OUT_DIR/zig_tree_build_loop.json.syms.json" \
    --out-dir "$OUT_DIR" \
    --label zig_tree_build_loop \
    --weight "$WEIGHT"
fi
