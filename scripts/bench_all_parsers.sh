#!/usr/bin/env bash
# Comprehensive EVTX parser benchmark script
# Compares: Rust evtx_dump, C libevtx, Go Velocidex, Go 0xrawsec, Python python-evtx
#
# Usage: ./scripts/bench_all_parsers.sh [evtx_file]
# Default file: samples/security_big_sample.evtx

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
EXTERNAL_DIR="$ROOT_DIR/external"

# Default test file
EVTX_FILE="${1:-$ROOT_DIR/samples/security_big_sample.evtx}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Hyperfine settings
HYPERFINE_WARMUP=2
HYPERFINE_RUNS=5

log_info() { echo -e "${BLUE}[INFO]${NC} $*"; }
log_success() { echo -e "${GREEN}[OK]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

check_dependencies() {
    log_info "Checking dependencies..."

    local missing=()
    command -v hyperfine &>/dev/null || missing+=("hyperfine")
    command -v go &>/dev/null || missing+=("go")
    command -v uv &>/dev/null || missing+=("uv")
    command -v git &>/dev/null || missing+=("git")

    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_info "Install with: brew install ${missing[*]}"
        exit 1
    fi

    if [[ ! -f "$EVTX_FILE" ]]; then
        log_error "EVTX file not found: $EVTX_FILE"
        exit 1
    fi

    log_success "All dependencies found"
}

# ============================================================================
# Build Functions
# ============================================================================

build_rust_evtx() {
    log_info "Building Rust evtx_dump..."
    cd "$ROOT_DIR"
    if [[ ! -f "target/release/evtx_dump" ]] || [[ "$FORCE_REBUILD" == "1" ]]; then
        cargo build --release --bin evtx_dump 2>/dev/null
    fi
    log_success "Rust evtx_dump ready: target/release/evtx_dump"
}

fetch_and_build_libevtx() {
    log_info "Setting up C libevtx..."
    cd "$EXTERNAL_DIR"

    if [[ ! -d "libevtx" ]]; then
        log_info "Cloning libevtx..."
        git clone --quiet https://github.com/libyal/libevtx.git
    fi

    cd libevtx

    if [[ ! -f "evtxtools/evtxexport" ]] || [[ "$FORCE_REBUILD" == "1" ]]; then
        log_info "Building libevtx (this may take a while)..."

        # Sync dependencies
        if [[ ! -d "libcerror" ]]; then
            ./synclibs.sh 2>/dev/null
        fi

        # Generate configure if needed
        if [[ ! -f "configure" ]]; then
            ./autogen.sh 2>/dev/null
        fi

        # Configure and build
        ./configure --enable-static --disable-shared --enable-static-executables \
            --quiet 2>/dev/null
        make -j"$(sysctl -n hw.ncpu 2>/dev/null || nproc)" --quiet 2>/dev/null
    fi

    log_success "C libevtx ready: external/libevtx/evtxtools/evtxexport"
}

fetch_and_build_velocidex() {
    log_info "Setting up Go Velocidex evtx..."
    cd "$EXTERNAL_DIR"

    if [[ ! -d "velocidex-evtx" ]]; then
        log_info "Cloning Velocidex evtx..."
        git clone --quiet https://github.com/Velocidex/evtx.git velocidex-evtx
    fi

    cd velocidex-evtx

    if [[ ! -f "dumpevtx" ]] || [[ "$FORCE_REBUILD" == "1" ]]; then
        log_info "Building Velocidex dumpevtx..."
        go build -o dumpevtx ./cmd/ 2>/dev/null
    fi

    log_success "Go Velocidex ready: external/velocidex-evtx/dumpevtx"
}

fetch_and_build_0xrawsec() {
    log_info "Setting up Go 0xrawsec evtx..."
    cd "$EXTERNAL_DIR"

    if [[ ! -d "0xrawsec-evtx" ]]; then
        log_info "Cloning 0xrawsec evtx..."
        git clone --quiet https://github.com/0xrawsec/golang-evtx.git 0xrawsec-evtx
    fi

    cd 0xrawsec-evtx

    if [[ ! -f "evtxdump" ]] || [[ "$FORCE_REBUILD" == "1" ]]; then
        log_info "Building 0xrawsec evtxdump..."

        # Fix missing Version/CommitID if needed
        if ! grep -q "Version.*=.*\"" tools/evtxdump/evtxdump.go 2>/dev/null; then
            sed -i.bak '/^const (/,/^)/{
                /conditions;`$/a\
	Version  = "dev"\
	CommitID = "unknown"
            }' tools/evtxdump/evtxdump.go 2>/dev/null || true
        fi

        go build -o evtxdump ./tools/evtxdump/ 2>/dev/null
    fi

    log_success "Go 0xrawsec ready: external/0xrawsec-evtx/evtxdump"
}

fetch_and_setup_python_evtx() {
    log_info "Setting up Python python-evtx..."
    cd "$EXTERNAL_DIR"

    if [[ ! -d "python-evtx" ]]; then
        log_info "Cloning python-evtx..."
        git clone --quiet https://github.com/williballenthin/python-evtx.git
    fi

    cd python-evtx

    # Setup with CPython
    if [[ ! -d ".venv-cpython" ]]; then
        log_info "Setting up CPython venv..."
        uv venv --python 3.13 .venv-cpython 2>/dev/null
        uv pip install --quiet -p .venv-cpython/bin/python -e . 2>/dev/null
    fi

    # Setup with PyPy
    if [[ ! -d ".venv-pypy" ]]; then
        log_info "Setting up PyPy venv..."
        uv python install pypy3.10 2>/dev/null || true
        if uv venv --python pypy3.10 .venv-pypy 2>/dev/null; then
            uv pip install --quiet -p .venv-pypy/bin/python -e . 2>/dev/null
        else
            log_warn "PyPy setup failed, skipping"
        fi
    fi

    log_success "Python python-evtx ready"
}

# ============================================================================
# Benchmark Functions
# ============================================================================

count_events() {
    local file="$1"
    local pattern="${2:-</Event>}"
    grep -c "$pattern" "$file" 2>/dev/null || echo "0"
}

benchmark_rust() {
    local bin="$ROOT_DIR/target/release/evtx_dump"
    [[ ! -x "$bin" ]] && return 1

    echo ""
    log_info "=== Rust evtx_dump ==="

    # XML output
    echo "  XML output:"
    hyperfine --warmup "$HYPERFINE_WARMUP" --runs "$HYPERFINE_RUNS" \
        --export-json /tmp/rust_xml.json \
        "$bin '$EVTX_FILE' > /dev/null" 2>&1 | sed 's/^/    /'

    # JSON output
    echo "  JSON output:"
    hyperfine --warmup "$HYPERFINE_WARMUP" --runs "$HYPERFINE_RUNS" \
        --export-json /tmp/rust_json.json \
        "$bin -o json '$EVTX_FILE' > /dev/null" 2>&1 | sed 's/^/    /'

    # Event count
    local events
    events=$("$bin" "$EVTX_FILE" 2>/dev/null | grep -c '</Event>' || echo "0")
    echo "  Events parsed: $events"
}

benchmark_libevtx() {
    local bin="$EXTERNAL_DIR/libevtx/evtxtools/evtxexport"
    [[ ! -x "$bin" ]] && return 1

    echo ""
    log_info "=== C libevtx (evtxexport) ==="

    # Default output (text format)
    echo "  Text output:"
    hyperfine --warmup "$HYPERFINE_WARMUP" --runs "$HYPERFINE_RUNS" \
        --export-json /tmp/libevtx.json \
        "$bin '$EVTX_FILE' > /dev/null" 2>&1 | sed 's/^/    /'

    # XML output
    echo "  XML output (-f xml):"
    hyperfine --warmup "$HYPERFINE_WARMUP" --runs "$HYPERFINE_RUNS" \
        "$bin -f xml '$EVTX_FILE' > /dev/null" 2>&1 | sed 's/^/    /'

    # Event count
    local events
    events=$("$bin" "$EVTX_FILE" 2>/dev/null | grep -c '^Event number' || echo "0")
    echo "  Events parsed: $events"
}

benchmark_velocidex() {
    local bin="$EXTERNAL_DIR/velocidex-evtx/dumpevtx"
    [[ ! -x "$bin" ]] && return 1

    echo ""
    log_info "=== Go Velocidex (dumpevtx) ==="

    # JSON output (default)
    echo "  JSON output:"
    hyperfine --warmup "$HYPERFINE_WARMUP" --runs "$HYPERFINE_RUNS" \
        --export-json /tmp/velocidex.json \
        "$bin parse '$EVTX_FILE' > /dev/null" 2>&1 | sed 's/^/    /'

    # Event count
    local events
    events=$("$bin" parse "$EVTX_FILE" 2>/dev/null | grep -c '"EventRecordID"' || echo "0")
    echo "  Events parsed: $events"
}

benchmark_0xrawsec() {
    local bin="$EXTERNAL_DIR/0xrawsec-evtx/evtxdump"
    [[ ! -x "$bin" ]] && return 1

    echo ""
    log_info "=== Go 0xrawsec (evtxdump) ==="

    # JSON output (default)
    echo "  JSON output:"
    hyperfine --warmup "$HYPERFINE_WARMUP" --runs "$HYPERFINE_RUNS" \
        --export-json /tmp/0xrawsec.json \
        "$bin '$EVTX_FILE' > /dev/null" 2>&1 | sed 's/^/    /'

    # Event count
    local events
    events=$("$bin" "$EVTX_FILE" 2>/dev/null | grep -c '"EventRecordID"' || echo "0")
    echo "  Events parsed: $events"
}

benchmark_python_cpython() {
    local venv="$EXTERNAL_DIR/python-evtx/.venv-cpython"
    local script="$EXTERNAL_DIR/python-evtx/evtx_scripts/evtx_dump.py"
    [[ ! -d "$venv" ]] && return 1

    echo ""
    log_info "=== Python (CPython) - single run ==="

    local python="$venv/bin/python"
    echo "  Python version: $($python --version 2>&1)"

    echo "  XML output (with JIT):"
    local start end elapsed
    start=$(date +%s.%N)
    PYTHON_JIT=1 "$python" "$script" "$EVTX_FILE" > /tmp/python_cpython.xml 2>&1
    end=$(date +%s.%N)
    elapsed=$(echo "$end - $start" | bc)
    echo "    Time: ${elapsed}s"

    local events
    events=$(grep -c '</Event>' /tmp/python_cpython.xml 2>/dev/null || echo "0")
    echo "  Events parsed: $events"
}

benchmark_python_pypy() {
    local venv="$EXTERNAL_DIR/python-evtx/.venv-pypy"
    local script="$EXTERNAL_DIR/python-evtx/evtx_scripts/evtx_dump.py"
    [[ ! -d "$venv" ]] && return 1

    echo ""
    log_info "=== Python (PyPy) - single run ==="

    local python="$venv/bin/python"
    echo "  Python version: $($python --version 2>&1)"

    echo "  XML output:"
    local start end elapsed
    start=$(date +%s.%N)
    "$python" "$script" "$EVTX_FILE" > /tmp/python_pypy.xml 2>&1
    end=$(date +%s.%N)
    elapsed=$(echo "$end - $start" | bc)
    echo "    Time: ${elapsed}s"

    local events
    events=$(grep -c '</Event>' /tmp/python_pypy.xml 2>/dev/null || echo "0")
    echo "  Events parsed: $events"
}

run_comparison_benchmark() {
    echo ""
    log_info "=== Head-to-head comparison (hyperfine) ==="

    local cmds=()
    local names=()

    # Rust
    if [[ -x "$ROOT_DIR/target/release/evtx_dump" ]]; then
        cmds+=("$ROOT_DIR/target/release/evtx_dump '$EVTX_FILE' > /dev/null")
        names+=("Rust evtx_dump")
    fi

    # C libevtx
    if [[ -x "$EXTERNAL_DIR/libevtx/evtxtools/evtxexport" ]]; then
        cmds+=("$EXTERNAL_DIR/libevtx/evtxtools/evtxexport '$EVTX_FILE' > /dev/null")
        names+=("C libevtx")
    fi

    # Go Velocidex
    if [[ -x "$EXTERNAL_DIR/velocidex-evtx/dumpevtx" ]]; then
        cmds+=("$EXTERNAL_DIR/velocidex-evtx/dumpevtx parse '$EVTX_FILE' > /dev/null")
        names+=("Go Velocidex")
    fi

    # Go 0xrawsec
    if [[ -x "$EXTERNAL_DIR/0xrawsec-evtx/evtxdump" ]]; then
        cmds+=("$EXTERNAL_DIR/0xrawsec-evtx/evtxdump '$EVTX_FILE' > /dev/null")
        names+=("Go 0xrawsec")
    fi

    if [[ ${#cmds[@]} -lt 2 ]]; then
        log_warn "Not enough implementations to compare"
        return 1
    fi

    local hyperfine_args=()
    for i in "${!cmds[@]}"; do
        hyperfine_args+=("-n" "${names[$i]}" "${cmds[$i]}")
    done

    hyperfine --warmup "$HYPERFINE_WARMUP" --runs "$HYPERFINE_RUNS" \
        --export-markdown /tmp/benchmark_comparison.md \
        "${hyperfine_args[@]}"

    echo ""
    echo "Markdown results saved to: /tmp/benchmark_comparison.md"
    cat /tmp/benchmark_comparison.md
}

print_summary() {
    echo ""
    echo "============================================================================"
    log_info "BENCHMARK SUMMARY"
    echo "============================================================================"
    echo "Test file: $EVTX_FILE"
    echo "File size: $(du -h "$EVTX_FILE" | cut -f1)"
    echo ""
    echo "Results saved to /tmp/benchmark_comparison.md"
    echo ""
    echo "Note: Python implementations run only once due to long execution time."
    echo "      All other implementations run with hyperfine ($HYPERFINE_RUNS runs, $HYPERFINE_WARMUP warmup)."
}

# ============================================================================
# Main
# ============================================================================

main() {
    FORCE_REBUILD="${FORCE_REBUILD:-0}"

    echo "============================================================================"
    echo "EVTX Parser Benchmark Suite"
    echo "============================================================================"
    echo ""

    check_dependencies
    mkdir -p "$EXTERNAL_DIR"

    # Build all implementations
    echo ""
    log_info "Building implementations..."
    build_rust_evtx
    fetch_and_build_libevtx
    fetch_and_build_velocidex
    fetch_and_build_0xrawsec
    fetch_and_setup_python_evtx

    # Run benchmarks
    echo ""
    echo "============================================================================"
    log_info "Running benchmarks..."
    echo "============================================================================"

    benchmark_rust
    benchmark_libevtx
    benchmark_velocidex
    benchmark_0xrawsec
    benchmark_python_cpython
    benchmark_python_pypy

    # Head-to-head comparison
    run_comparison_benchmark

    print_summary
}

main "$@"
