#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

FILE="$ROOT/samples/security_big_sample.evtx"
THREADS=1
WARMUP=10
RUNS=10
PROFILE_LOOPS=20
WEIGHT="cpu"
OUT_DIR="$ROOT/perf"
SKIP_BUILD=0
SKIP_SAMPLY=0
SKIP_HYPERFINE=0
NO_REDIRECT=0
RUST_FEATURES="fast-alloc"

ZIG_ROOT="${ZIG_ROOT:-/Users/omerba/Workspace/zig-evtx}"
ZIG_BIN="${ZIG_BIN:-$ZIG_ROOT/zig-out/bin/evtx_dump_zig}"
ZIG_LOOP_BIN="${ZIG_LOOP_BIN:-$ZIG_ROOT/zig-out/bin/evtx_dump_loop}"

usage() {
  cat <<'EOF'
Usage: scripts/perf_evtx_dump.sh [options]

Options:
  --file <path>          EVTX input file (default: samples/security_big_sample.evtx)
  --threads <n>          Threads for evtx_dump (default: 1)
  --warmup <n>           Hyperfine warmup runs (default: 10)
  --runs <n>             Hyperfine runs (default: 10)
  --profile-loops <n>    Loop count for samply profile (default: 20)
  --no-redirect          Do not redirect output to /dev/null
  --features <list>      Cargo features (default: fast-alloc; `bench` auto-added unless --skip-samply)
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
    --threads) THREADS="$2"; shift 2;;
    --warmup) WARMUP="$2"; shift 2;;
    --runs) RUNS="$2"; shift 2;;
    --profile-loops) PROFILE_LOOPS="$2"; shift 2;;
    --no-redirect) NO_REDIRECT=1; shift;;
    --features) RUST_FEATURES="$2"; shift 2;;
    --weight) WEIGHT="$2"; shift 2;;
    --out-dir) OUT_DIR="$2"; shift 2;;
    --skip-build) SKIP_BUILD=1; shift;;
    --skip-hyperfine) SKIP_HYPERFINE=1; shift;;
    --skip-samply) SKIP_SAMPLY=1; shift;;
    -h|--help) usage; exit 0;;
    *) echo "Unknown arg: $1"; usage; exit 1;;
  esac
done

RUST_BIN="$ROOT/target/release/evtx_dump"
RUST_LOOP_BIN="$ROOT/target/release/bench_evtx_dump_loop"

# `bench_evtx_dump_loop` is gated behind the `bench` feature.
if [[ $SKIP_SAMPLY -eq 0 && " $RUST_FEATURES " != *" bench "* ]]; then
  RUST_FEATURES="${RUST_FEATURES} bench"
fi

REDIRECT="> /dev/null"
if [[ $NO_REDIRECT -eq 1 ]]; then
  REDIRECT=""
fi

mkdir -p "$OUT_DIR"

if [[ $SKIP_BUILD -eq 0 ]]; then
  if [[ -n "$RUST_FEATURES" ]]; then
    (cd "$ROOT" && cargo build --release --features "$RUST_FEATURES" --bin evtx_dump --bin bench_evtx_dump_loop)
  else
    (cd "$ROOT" && cargo build --release --bin evtx_dump --bin bench_evtx_dump_loop)
  fi
  (cd "$ZIG_ROOT" && zig build -Doptimize=ReleaseFast)
fi

RUST_CMD="$RUST_BIN -t $THREADS -o jsonl $FILE"
ZIG_CMD="$ZIG_BIN --no-checks -t $THREADS -o jsonl $FILE"

if [[ $SKIP_HYPERFINE -eq 0 ]]; then
  hyperfine -w "$WARMUP" -r "$RUNS" \
    "$RUST_CMD $REDIRECT" \
    "$ZIG_CMD $REDIRECT" \
    --export-json "$OUT_DIR/hyperfine_evtx_dump.json" \
    | tee "$OUT_DIR/hyperfine_evtx_dump.txt"
fi

if [[ $SKIP_SAMPLY -eq 0 ]]; then
  samply record --save-only --unstable-presymbolicate \
    -o "$OUT_DIR/rust_evtx_dump.json.gz" -- \
    "$RUST_LOOP_BIN" --file "$FILE" --loops "$PROFILE_LOOPS" --threads "$THREADS"

  if [[ -x "$ZIG_BIN" && -x "$ZIG_LOOP_BIN" ]]; then
    samply record --save-only --unstable-presymbolicate \
      -o "$OUT_DIR/zig_evtx_dump.json.gz" -- \
      "$ZIG_LOOP_BIN" --file "$FILE" --loops "$PROFILE_LOOPS" --threads "$THREADS"
  else
    echo "Skipping Zig samply profile (set ZIG_LOOP_BIN to a loop-capable binary)." >&2
  fi

  python3 "$ROOT/scripts/samply_extract_tables.py" \
    --profile "$OUT_DIR/rust_evtx_dump.json.gz" \
    --syms "$OUT_DIR/rust_evtx_dump.json.syms.json" \
    --out-dir "$OUT_DIR" \
    --label rust_evtx_dump \
    --weight "$WEIGHT"

  if [[ -f "$OUT_DIR/zig_evtx_dump.json.gz" ]]; then
    python3 "$ROOT/scripts/samply_extract_tables.py" \
      --profile "$OUT_DIR/zig_evtx_dump.json.gz" \
      --syms "$OUT_DIR/zig_evtx_dump.json.syms.json" \
      --out-dir "$OUT_DIR" \
      --label zig_evtx_dump \
      --weight "$WEIGHT"
  fi
fi
