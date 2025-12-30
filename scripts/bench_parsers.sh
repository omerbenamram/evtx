#!/usr/bin/env bash
#
# Comprehensive EVTX Parser Benchmark
# Compares: evtx (Rust), libevtx (C), python-evtx, golang-evtx, velocidex/evtx
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_DIR="${BENCH_DIR:-$ROOT/benchmark_parsers}"
SAMPLES_DIR="${SAMPLES_DIR:-$ROOT/samples}"
RESULTS_DIR="${RESULTS_DIR:-$ROOT/benchmark_results}"

# Default settings
EVTX_FILE=""
WARMUP=3
RUNS=10
THREADS_LIST="1 8"  # Space-separated list of thread counts
SKIP_CLONE=0
SKIP_BUILD=0
SKIP_RUST=0
SKIP_LIBEVTX=0
SKIP_PYTHON=0
SKIP_GOLANG_EVTX=0
SKIP_VELOCIDEX=0
USE_PYPY=0
MAX_THREADS=""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${BLUE}[INFO]${NC} $*"; }
log_ok() { echo -e "${GREEN}[OK]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_err() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

usage() {
  cat <<'EOF'
Usage: scripts/bench_parsers.sh [options]

Benchmark evtx against other EVTX parsers (libevtx, python-evtx, golang-evtx, velocidex/evtx)

Options:
  --file <path>           EVTX input file (required, ~30MB recommended)
  --warmup <n>            Hyperfine warmup runs (default: 3)
  --runs <n>              Hyperfine runs (default: 10)
  --threads <list>        Space-separated thread counts (default: "1 8")
  --max-threads           Include $(nproc) threads in the list
  --bench-dir <dir>       Directory for cloned repos (default: benchmark_parsers/)
  --results-dir <dir>     Output directory for results (default: benchmark_results/)

  --skip-clone            Don't clone repos (assume already present)
  --skip-build            Don't rebuild (assume already built)
  --skip-rust             Skip Rust evtx benchmarks
  --skip-libevtx          Skip libevtx benchmarks
  --skip-python           Skip python-evtx benchmarks
  --skip-golang-evtx      Skip golang-evtx benchmarks
  --skip-velocidex        Skip velocidex/evtx benchmarks
  --use-pypy              Also benchmark with PyPy (if installed)

  -h, --help              Show this help

Examples:
  # Basic benchmark with a sample file
  ./scripts/bench_parsers.sh --file samples/security.evtx

  # Full benchmark with max threads
  ./scripts/bench_parsers.sh --file samples/big.evtx --max-threads --threads "1 8 24"

  # Quick test (fewer runs)
  ./scripts/bench_parsers.sh --file samples/test.evtx --warmup 1 --runs 3

Sample EVTX files:
  You can download sample .evtx files from:
  - https://github.com/sbousseaden/EVTX-ATTACK-SAMPLES
  - https://github.com/NextronSystems/evtx-baseline

EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --file) EVTX_FILE="$2"; shift 2;;
    --warmup) WARMUP="$2"; shift 2;;
    --runs) RUNS="$2"; shift 2;;
    --threads) THREADS_LIST="$2"; shift 2;;
    --max-threads) MAX_THREADS=1; shift;;
    --bench-dir) BENCH_DIR="$2"; shift 2;;
    --results-dir) RESULTS_DIR="$2"; shift 2;;
    --skip-clone) SKIP_CLONE=1; shift;;
    --skip-build) SKIP_BUILD=1; shift;;
    --skip-rust) SKIP_RUST=1; shift;;
    --skip-libevtx) SKIP_LIBEVTX=1; shift;;
    --skip-python) SKIP_PYTHON=1; shift;;
    --skip-golang-evtx) SKIP_GOLANG_EVTX=1; shift;;
    --skip-velocidex) SKIP_VELOCIDEX=1; shift;;
    --use-pypy) USE_PYPY=1; shift;;
    -h|--help) usage; exit 0;;
    *) log_err "Unknown arg: $1"; usage; exit 1;;
  esac
done

# Validate required args
if [[ -z "$EVTX_FILE" ]]; then
  log_err "Missing required --file argument"
  echo ""
  usage
  exit 1
fi

if [[ ! -f "$EVTX_FILE" ]]; then
  log_err "EVTX file not found: $EVTX_FILE"
  exit 1
fi

# Add max threads if requested
if [[ -n "$MAX_THREADS" ]]; then
  NPROC=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 8)
  THREADS_LIST="$THREADS_LIST $NPROC"
fi

# Get file size for reporting
FILE_SIZE=$(du -h "$EVTX_FILE" | cut -f1)
log_info "Benchmarking with: $EVTX_FILE ($FILE_SIZE)"
log_info "Thread configurations: $THREADS_LIST"

# Check for hyperfine
if ! command -v hyperfine &>/dev/null; then
  log_err "hyperfine not found. Install it with: cargo install hyperfine"
  exit 1
fi

mkdir -p "$BENCH_DIR" "$RESULTS_DIR"

#############################################################################
# Clone Repositories
#############################################################################

clone_repo() {
  local name="$1"
  local url="$2"
  local dir="$BENCH_DIR/$name"
  
  if [[ -d "$dir" ]]; then
    log_info "$name already cloned"
    return 0
  fi
  
  log_info "Cloning $name..."
  git clone --depth 1 "$url" "$dir"
  log_ok "Cloned $name"
}

if [[ $SKIP_CLONE -eq 0 ]]; then
  log_info "=== Cloning repositories ==="
  
  [[ $SKIP_LIBEVTX -eq 0 ]] && clone_repo "libevtx" "https://github.com/libyal/libevtx.git"
  [[ $SKIP_PYTHON -eq 0 ]] && clone_repo "python-evtx" "https://github.com/williballenthin/python-evtx.git"
  [[ $SKIP_GOLANG_EVTX -eq 0 ]] && clone_repo "golang-evtx" "https://github.com/0xrawsec/golang-evtx.git"
  [[ $SKIP_VELOCIDEX -eq 0 ]] && clone_repo "velocidex-evtx" "https://github.com/Velocidex/evtx.git"
fi

#############################################################################
# Build: Rust evtx (this library)
#############################################################################

RUST_BIN="$ROOT/target/release/evtx_dump"

build_rust() {
  log_info "Building Rust evtx..."
  (cd "$ROOT" && cargo build --release --features "fast-alloc,multithreading")
  log_ok "Rust evtx built"
}

if [[ $SKIP_RUST -eq 0 && $SKIP_BUILD -eq 0 ]]; then
  build_rust
fi

#############################################################################
# Build: libevtx (C)
#############################################################################

LIBEVTX_BIN="$BENCH_DIR/libevtx/evtxtools/evtxexport"

build_libevtx() {
  local dir="$BENCH_DIR/libevtx"
  
  if [[ ! -d "$dir" ]]; then
    log_warn "libevtx not cloned, skipping build"
    return 1
  fi
  
  log_info "Building libevtx..."
  
  (cd "$dir"
    # libevtx requires autotools
    if [[ ! -f "configure" ]]; then
      if ! command -v autoreconf &>/dev/null; then
        log_err "autoreconf not found. Install autotools: brew install autoconf automake libtool"
        return 1
      fi
      ./synclibs.sh 2>/dev/null || true
      autoreconf -fiv
    fi
    
    if [[ ! -f "Makefile" ]]; then
      ./configure --enable-silent-rules
    fi
    
    make -j"$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)"
  )
  
  if [[ -x "$LIBEVTX_BIN" ]]; then
    log_ok "libevtx built"
    return 0
  else
    log_warn "libevtx build may have failed, binary not found"
    return 1
  fi
}

HAVE_LIBEVTX=0
if [[ $SKIP_LIBEVTX -eq 0 && $SKIP_BUILD -eq 0 ]]; then
  if build_libevtx; then
    HAVE_LIBEVTX=1
  fi
elif [[ $SKIP_LIBEVTX -eq 0 && -x "$LIBEVTX_BIN" ]]; then
  HAVE_LIBEVTX=1
fi

#############################################################################
# Build: python-evtx
#############################################################################

PYTHON_EVTX_SCRIPT="$BENCH_DIR/python-evtx/scripts/evtx_dump.py"

setup_python_evtx() {
  local dir="$BENCH_DIR/python-evtx"
  
  if [[ ! -d "$dir" ]]; then
    log_warn "python-evtx not cloned, skipping"
    return 1
  fi
  
  log_info "Setting up python-evtx..."
  
  # Create venv if not exists
  if [[ ! -d "$dir/venv" ]]; then
    python3 -m venv "$dir/venv"
  fi
  
  # Install
  (
    source "$dir/venv/bin/activate"
    pip install -q -e "$dir"
  )
  
  log_ok "python-evtx ready"
  return 0
}

HAVE_PYTHON_EVTX=0
if [[ $SKIP_PYTHON -eq 0 && $SKIP_BUILD -eq 0 ]]; then
  if setup_python_evtx; then
    HAVE_PYTHON_EVTX=1
  fi
elif [[ $SKIP_PYTHON -eq 0 && -f "$PYTHON_EVTX_SCRIPT" ]]; then
  HAVE_PYTHON_EVTX=1
fi

#############################################################################
# Build: golang-evtx (0xrawsec)
#############################################################################

GOLANG_EVTX_BIN="$BENCH_DIR/golang-evtx/evtxdump"

build_golang_evtx() {
  local dir="$BENCH_DIR/golang-evtx"
  
  if [[ ! -d "$dir" ]]; then
    log_warn "golang-evtx not cloned, skipping"
    return 1
  fi
  
  if ! command -v go &>/dev/null; then
    log_warn "Go not installed, skipping golang-evtx"
    return 1
  fi
  
  log_info "Building golang-evtx..."
  
  (cd "$dir"
    # Build the evtxdump tool
    go build -o evtxdump ./tools/evtxdump/
  )
  
  if [[ -x "$GOLANG_EVTX_BIN" ]]; then
    log_ok "golang-evtx built"
    return 0
  else
    log_warn "golang-evtx build failed"
    return 1
  fi
}

HAVE_GOLANG_EVTX=0
if [[ $SKIP_GOLANG_EVTX -eq 0 && $SKIP_BUILD -eq 0 ]]; then
  if build_golang_evtx; then
    HAVE_GOLANG_EVTX=1
  fi
elif [[ $SKIP_GOLANG_EVTX -eq 0 && -x "$GOLANG_EVTX_BIN" ]]; then
  HAVE_GOLANG_EVTX=1
fi

#############################################################################
# Build: velocidex/evtx (Go)
#############################################################################

VELOCIDEX_BIN="$BENCH_DIR/velocidex-evtx/bin/dump"

build_velocidex() {
  local dir="$BENCH_DIR/velocidex-evtx"
  
  if [[ ! -d "$dir" ]]; then
    log_warn "velocidex/evtx not cloned, skipping"
    return 1
  fi
  
  if ! command -v go &>/dev/null; then
    log_warn "Go not installed, skipping velocidex/evtx"
    return 1
  fi
  
  log_info "Building velocidex/evtx..."
  
  (cd "$dir"
    mkdir -p bin
    go build -o bin/dump ./bin/
  )
  
  if [[ -x "$VELOCIDEX_BIN" ]]; then
    log_ok "velocidex/evtx built"
    return 0
  else
    log_warn "velocidex/evtx build failed"
    return 1
  fi
}

HAVE_VELOCIDEX=0
if [[ $SKIP_VELOCIDEX -eq 0 && $SKIP_BUILD -eq 0 ]]; then
  if build_velocidex; then
    HAVE_VELOCIDEX=1
  fi
elif [[ $SKIP_VELOCIDEX -eq 0 && -x "$VELOCIDEX_BIN" ]]; then
  HAVE_VELOCIDEX=1
fi

#############################################################################
# Run Benchmarks
#############################################################################

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULT_PREFIX="$RESULTS_DIR/bench_${TIMESTAMP}"

log_info ""
log_info "=== Running Benchmarks ==="
log_info "File: $EVTX_FILE ($FILE_SIZE)"
log_info "Results will be saved to: $RESULTS_DIR/"
log_info ""

# Summary of what will be benchmarked
echo "Parsers to benchmark:"
[[ $SKIP_RUST -eq 0 ]] && echo "  ✓ Rust evtx (this library)"
[[ $HAVE_LIBEVTX -eq 1 ]] && echo "  ✓ libevtx (C)" || echo "  ✗ libevtx (not available)"
[[ $HAVE_PYTHON_EVTX -eq 1 ]] && echo "  ✓ python-evtx (CPython)" || echo "  ✗ python-evtx (not available)"
[[ $HAVE_GOLANG_EVTX -eq 1 ]] && echo "  ✓ golang-evtx (Go, multiprocessing)" || echo "  ✗ golang-evtx (not available)"
[[ $HAVE_VELOCIDEX -eq 1 ]] && echo "  ✓ velocidex/evtx (Go)" || echo "  ✗ velocidex/evtx (not available)"
echo ""

#############################################################################
# Benchmark: XML Output
#############################################################################

run_xml_benchmark() {
  local threads="$1"
  log_info "--- XML Benchmark (threads=$threads) ---"
  
  local cmds=()
  local names=()
  
  # Rust evtx (XML)
  if [[ $SKIP_RUST -eq 0 && -x "$RUST_BIN" ]]; then
    cmds+=("$RUST_BIN -t $threads -o xml '$EVTX_FILE' > /dev/null")
    names+=("evtx-rust-xml-t$threads")
  fi
  
  # libevtx (XML only, single-threaded)
  if [[ $HAVE_LIBEVTX -eq 1 && "$threads" == "1" ]]; then
    cmds+=("$LIBEVTX_BIN '$EVTX_FILE' > /dev/null")
    names+=("libevtx-xml")
  fi
  
  # python-evtx (XML, single-threaded, very slow)
  if [[ $HAVE_PYTHON_EVTX -eq 1 && "$threads" == "1" ]]; then
    local python_cmd="source $BENCH_DIR/python-evtx/venv/bin/activate && python $PYTHON_EVTX_SCRIPT '$EVTX_FILE' > /dev/null"
    # Only include python-evtx for small files or explicit request
    cmds+=("bash -c \"$python_cmd\"")
    names+=("python-evtx-xml")
  fi
  
  if [[ ${#cmds[@]} -eq 0 ]]; then
    log_warn "No XML benchmarks to run for threads=$threads"
    return
  fi
  
  local hyperfine_args=()
  for i in "${!cmds[@]}"; do
    hyperfine_args+=(-n "${names[$i]}" "${cmds[$i]}")
  done
  
  hyperfine -w "$WARMUP" -r "$RUNS" \
    "${hyperfine_args[@]}" \
    --export-json "${RESULT_PREFIX}_xml_t${threads}.json" \
    --export-markdown "${RESULT_PREFIX}_xml_t${threads}.md" \
    | tee "${RESULT_PREFIX}_xml_t${threads}.txt"
}

#############################################################################
# Benchmark: JSON Output
#############################################################################

run_json_benchmark() {
  local threads="$1"
  log_info "--- JSON Benchmark (threads=$threads) ---"
  
  local cmds=()
  local names=()
  
  # Rust evtx (JSON)
  if [[ $SKIP_RUST -eq 0 && -x "$RUST_BIN" ]]; then
    cmds+=("$RUST_BIN -t $threads -o jsonl '$EVTX_FILE' > /dev/null")
    names+=("evtx-rust-json-t$threads")
  fi
  
  # golang-evtx (JSON, uses multiprocessing internally)
  if [[ $HAVE_GOLANG_EVTX -eq 1 ]]; then
    cmds+=("$GOLANG_EVTX_BIN '$EVTX_FILE' > /dev/null")
    names+=("golang-evtx-json")
  fi
  
  # velocidex/evtx (JSON only, single-threaded)
  if [[ $HAVE_VELOCIDEX -eq 1 && "$threads" == "1" ]]; then
    cmds+=("$VELOCIDEX_BIN '$EVTX_FILE' > /dev/null")
    names+=("velocidex-evtx-json")
  fi
  
  if [[ ${#cmds[@]} -eq 0 ]]; then
    log_warn "No JSON benchmarks to run for threads=$threads"
    return
  fi
  
  local hyperfine_args=()
  for i in "${!cmds[@]}"; do
    hyperfine_args+=(-n "${names[$i]}" "${cmds[$i]}")
  done
  
  hyperfine -w "$WARMUP" -r "$RUNS" \
    "${hyperfine_args[@]}" \
    --export-json "${RESULT_PREFIX}_json_t${threads}.json" \
    --export-markdown "${RESULT_PREFIX}_json_t${threads}.md" \
    | tee "${RESULT_PREFIX}_json_t${threads}.txt"
}

#############################################################################
# Main benchmark loop
#############################################################################

for threads in $THREADS_LIST; do
  run_xml_benchmark "$threads"
  run_json_benchmark "$threads"
done

#############################################################################
# Generate Summary
#############################################################################

log_info ""
log_info "=== Benchmark Complete ==="
log_info "Results saved to: $RESULTS_DIR/"
log_info ""

# Create summary markdown
SUMMARY_FILE="${RESULT_PREFIX}_summary.md"
{
  echo "# EVTX Parser Benchmark Results"
  echo ""
  echo "**Date:** $(date)"
  echo "**File:** $EVTX_FILE ($FILE_SIZE)"
  echo "**System:** $(uname -srm)"
  echo "**CPU:** $(sysctl -n machdep.cpu.brand_string 2>/dev/null || lscpu 2>/dev/null | grep 'Model name' | cut -d: -f2 | xargs || echo 'Unknown')"
  echo ""
  echo "## Results"
  echo ""
  
  for f in "${RESULT_PREFIX}"_*.md; do
    if [[ -f "$f" && "$f" != "$SUMMARY_FILE" ]]; then
      echo "### $(basename "$f" .md | sed 's/_/ /g')"
      echo ""
      cat "$f"
      echo ""
    fi
  done
} > "$SUMMARY_FILE"

log_ok "Summary written to: $SUMMARY_FILE"

# Print quick summary
echo ""
echo "=== Quick Results ==="
for f in "${RESULT_PREFIX}"_*.txt; do
  [[ -f "$f" ]] && cat "$f"
done
