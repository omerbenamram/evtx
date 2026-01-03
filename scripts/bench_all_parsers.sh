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

# Hyperfine settings (override via env)
HYPERFINE_WARMUP="${HYPERFINE_WARMUP:-2}"
HYPERFINE_RUNS="${HYPERFINE_RUNS:-5}"

# Thread settings (README grid)
THREADS_1="${THREADS_1:-1}"
THREADS_8="${THREADS_8:-8}"
THREADS_MAX="${THREADS_MAX:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 1)}"
THREADS_LIST="${THREADS_LIST:-$THREADS_1 $THREADS_8 $THREADS_MAX}"

# Optional extras
RUN_COMPARISON="${RUN_COMPARISON:-0}"

# Captured results (for README table)
declare -A BENCH_RESULTS=()
declare -A BENCH_META=()

log_info() { echo -e "${BLUE}[INFO]${NC} $*"; }
log_success() { echo -e "${GREEN}[OK]${NC} $*"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*"; }

normalize_threads_list() {
    # Print a de-duplicated, space-separated list of positive integers.
    # shellcheck disable=SC2206
    local arr=($*)
    local seen=" "
    local out=()
    local t
    for t in "${arr[@]}"; do
        [[ "$t" =~ ^[0-9]+$ ]] || continue
        [[ "$t" -gt 0 ]] || continue
        if [[ "$seen" != *" $t "* ]]; then
            out+=("$t")
            seen+=" $t "
        fi
    done
    echo "${out[*]}"
}

hf_format_json() {
    local json_file="$1"
    python3 - "$json_file" <<'PY'
import json
import sys

path = sys.argv[1]
data = json.load(open(path, "r", encoding="utf-8"))
res = data["results"][0]
mean = float(res["mean"])
std = float(res.get("stddev") or 0.0)

def fmt_pair(m: float, s: float) -> str:
    # Roughly matches hyperfine's human formatting; good enough for README tables.
    if m < 1.0:
        return f"{m*1000:.1f} ms ± {s*1000:.1f} ms"
    return f"{m:.3f} s ± {s:.3f} s"

print(fmt_pair(mean, std))
PY
}

hf_run() {
    local key="$1"
    local cmd="$2"
    local json_out="/tmp/${key}.json"

    hyperfine --warmup "$HYPERFINE_WARMUP" --runs "$HYPERFINE_RUNS" \
        --export-json "$json_out" \
        "$cmd" 2>&1 | sed 's/^/    /'

    BENCH_RESULTS["$key"]="$(hf_format_json "$json_out")"
}

measure_cmd_once_seconds() {
    local cmd="$1"
    python3 - "$cmd" <<'PY'
import subprocess
import sys
import time

cmd = sys.argv[1]
start = time.perf_counter()
proc = subprocess.run(cmd, shell=True)
end = time.perf_counter()
if proc.returncode != 0:
    raise SystemExit(proc.returncode)
print(f"{end - start:.6f}")
PY
}

format_seconds_human() {
    local seconds="$1"
    python3 - "$seconds" <<'PY'
import sys

s = float(sys.argv[1])
if s >= 60.0:
    m = int(s // 60.0)
    rem = s - (m * 60.0)
    print(f"{m}m{rem:.3f}s")
else:
    print(f"{s:.3f}s")
PY
}

binary_looks_compatible() {
    # When syncing a working tree across OSes, we can end up with stale binaries
    # from the other platform (e.g. Mach-O on Linux). Detect and rebuild.
    local bin="$1"
    [[ -f "$bin" ]] || return 1

    if ! command -v file >/dev/null 2>&1; then
        return 0
    fi

    local os desc
    os="$(uname -s 2>/dev/null || echo unknown)"
    desc="$(file "$bin" 2>/dev/null || true)"

    case "$os" in
        Linux)
            [[ "$desc" == *"Mach-O"* ]] && return 1
            ;;
        Darwin)
            [[ "$desc" == *"ELF"* ]] && return 1
            ;;
    esac

    return 0
}

check_dependencies() {
    log_info "Checking dependencies..."

    local missing=()
    command -v hyperfine &>/dev/null || missing+=("hyperfine")
    command -v go &>/dev/null || missing+=("go")
    command -v uv &>/dev/null || missing+=("uv")
    command -v git &>/dev/null || missing+=("git")
    command -v python3 &>/dev/null || missing+=("python3")

    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_info "Install with your package manager (e.g. brew/pacman/apt) or rustup/cargo as appropriate."
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

    if [[ ! -f "evtxtools/evtxexport" ]] || [[ "$FORCE_REBUILD" == "1" ]] || ! binary_looks_compatible "evtxtools/evtxexport"; then
        log_info "Building libevtx (this may take a while)..."
        rm -f evtxtools/evtxexport 2>/dev/null || true

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

    if [[ ! -f "dumpevtx" ]] || [[ "$FORCE_REBUILD" == "1" ]] || ! binary_looks_compatible "dumpevtx"; then
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

    if [[ ! -f "evtxdump" ]] || [[ "$FORCE_REBUILD" == "1" ]] || ! binary_looks_compatible "evtxdump"; then
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

    local threads
    for threads in $(normalize_threads_list "$THREADS_LIST"); do
        echo "  XML output (threads=$threads):"
        hf_run "rust_xml_t${threads}" "$bin -t $threads \"$EVTX_FILE\" > /dev/null"
    done

    for threads in $(normalize_threads_list "$THREADS_LIST"); do
        echo "  JSON output (threads=$threads):"
        hf_run "rust_json_t${threads}" "$bin -t $threads -o json \"$EVTX_FILE\" > /dev/null"
    done
}

benchmark_libevtx() {
    local bin="$EXTERNAL_DIR/libevtx/evtxtools/evtxexport"
    [[ ! -x "$bin" ]] && return 1

    echo ""
    log_info "=== C libevtx (evtxexport) ==="

    # XML output
    echo "  XML output (-f xml):"
    hf_run "libevtx_xml" "$bin -f xml \"$EVTX_FILE\" > /dev/null"
}

benchmark_velocidex() {
    local bin="$EXTERNAL_DIR/velocidex-evtx/dumpevtx"
    [[ ! -x "$bin" ]] && return 1

    echo ""
    log_info "=== Go Velocidex (dumpevtx) ==="

    # JSON output (default)
    echo "  JSON output:"
    hf_run "velocidex_json" "$bin parse \"$EVTX_FILE\" > /dev/null"
}

benchmark_0xrawsec() {
    local bin="$EXTERNAL_DIR/0xrawsec-evtx/evtxdump"
    [[ ! -x "$bin" ]] && return 1

    echo ""
    log_info "=== Go 0xrawsec (evtxdump) ==="

    # JSON output (default)
    echo "  JSON output:"
    hf_run "0xrawsec_json" "$bin \"$EVTX_FILE\" > /dev/null"
}

benchmark_python_cpython() {
    local venv="$EXTERNAL_DIR/python-evtx/.venv-cpython"
    local script="$EXTERNAL_DIR/python-evtx/evtx_scripts/evtx_dump.py"
    [[ ! -d "$venv" ]] && return 1

    echo ""
    log_info "=== Python (CPython) - single run ==="

    local python="$venv/bin/python"
    local pyver
    pyver="$($python -c 'import platform; print(f"{platform.python_implementation()} {platform.python_version()}")' 2>/dev/null || $python --version 2>&1)"
    echo "  Python version: $pyver"
    BENCH_META["python_cpython_version"]="$pyver"

    echo "  XML output (ran once):"
    local seconds
    seconds="$(measure_cmd_once_seconds "PYTHON_JIT=1 \"$python\" \"$script\" \"$EVTX_FILE\" > /dev/null")"
    BENCH_RESULTS["python_evtx_cpython_xml"]="$(format_seconds_human "$seconds") (ran once)"
    echo "    Time: ${BENCH_RESULTS["python_evtx_cpython_xml"]}"
}

benchmark_python_pypy() {
    local venv="$EXTERNAL_DIR/python-evtx/.venv-pypy"
    local script="$EXTERNAL_DIR/python-evtx/evtx_scripts/evtx_dump.py"
    [[ ! -d "$venv" ]] && return 1

    echo ""
    log_info "=== Python (PyPy) - single run ==="

    local python="$venv/bin/python"
    local pyver
    pyver="$($python -c 'import platform, sys; v=sys.pypy_version_info; print(f"PyPy {v.major}.{v.minor}.{v.micro} (Python {platform.python_version()})")' 2>/dev/null || $python --version 2>&1 | tr '\n' ' ')"
    echo "  Python version: $pyver"
    BENCH_META["python_pypy_version"]="$pyver"

    echo "  XML output (ran once):"
    local seconds
    seconds="$(measure_cmd_once_seconds "\"$python\" \"$script\" \"$EVTX_FILE\" > /dev/null")"
    BENCH_RESULTS["python_evtx_pypy_xml"]="$(format_seconds_human "$seconds") (ran once)"
    echo "    Time: ${BENCH_RESULTS["python_evtx_pypy_xml"]}"
}

benchmark_pyevtx_rs() {
    echo ""
    log_info "=== pyevtx-rs (PyPI: evtx) - single run ==="

    local pyver
    pyver="$(uv run --with evtx python -c 'import platform; print(f"{platform.python_implementation()} {platform.python_version()}")' 2>/dev/null || true)"
    [[ -n "$pyver" ]] && echo "  Python version: $pyver"
    BENCH_META["pyevtx_rs_python_version"]="$pyver"

    # Warm the uv cache/env so timing doesn't include install/resolution.
    uv run --with evtx python -c 'import evtx; print("warm")' >/dev/null 2>&1 || true

    echo "  XML output (ran once):"
    local seconds_xml
    seconds_xml="$(measure_cmd_once_seconds "uv run --with evtx python -c 'import sys, collections; from evtx import PyEvtxParser; p=PyEvtxParser(sys.argv[1]); collections.deque((sys.stdout.write(r[\"data\"] + \"\\n\") for r in p.records()), maxlen=0)' \"$EVTX_FILE\" > /dev/null")"
    BENCH_RESULTS["pyevtx_rs_xml"]="$(format_seconds_human "$seconds_xml") (ran once)"
    echo "    Time: ${BENCH_RESULTS["pyevtx_rs_xml"]}"

    echo "  JSON output (ran once):"
    local seconds_json
    seconds_json="$(measure_cmd_once_seconds "uv run --with evtx python -c 'import sys, collections; from evtx import PyEvtxParser; p=PyEvtxParser(sys.argv[1]); collections.deque((sys.stdout.write(r[\"data\"] + \"\\n\") for r in p.records_json()), maxlen=0)' \"$EVTX_FILE\" > /dev/null")"
    BENCH_RESULTS["pyevtx_rs_json"]="$(format_seconds_human "$seconds_json") (ran once)"
    echo "    Time: ${BENCH_RESULTS["pyevtx_rs_json"]}"
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

write_readme_table() {
    local tmax="$THREADS_MAX"
    local py_cpython="${BENCH_META["python_cpython_version"]:-CPython}"
    local py_pypy="${BENCH_META["python_pypy_version"]:-PyPy}"
    local py_pyevtx="${BENCH_META["pyevtx_rs_python_version"]:-Python}"

    local out="${README_TABLE_OUT:-/tmp/evtx_bench_readme_table.md}"

    {
        echo "|                  | evtx (1 thread) | evtx (8 threads) | evtx (${tmax} threads) | libevtx (C) | velocidex/evtx (go) | golang-evtx (uses multiprocessing) | pyevtx-rs ($py_pyevtx) | python-evtx ($py_cpython) | python-evtx ($py_pypy) |"
        echo "|------------------|-----------------|------------------|------------------------|-------------|----------------------|------------------------------------|------------------------|---------------------------|------------------------|"
        echo "| 30MB evtx (XML)  | ${BENCH_RESULTS["rust_xml_t1"]:-N/A} | ${BENCH_RESULTS["rust_xml_t8"]:-N/A} | ${BENCH_RESULTS["rust_xml_t${tmax}"]:-N/A} | ${BENCH_RESULTS["libevtx_xml"]:-N/A} | No support | No support | ${BENCH_RESULTS["pyevtx_rs_xml"]:-N/A} | ${BENCH_RESULTS["python_evtx_cpython_xml"]:-N/A} | ${BENCH_RESULTS["python_evtx_pypy_xml"]:-N/A} |"
        echo "| 30MB evtx (JSON) | ${BENCH_RESULTS["rust_json_t1"]:-N/A} | ${BENCH_RESULTS["rust_json_t8"]:-N/A} | ${BENCH_RESULTS["rust_json_t${tmax}"]:-N/A} | No support | ${BENCH_RESULTS["velocidex_json"]:-N/A} | ${BENCH_RESULTS["0xrawsec_json"]:-N/A} | ${BENCH_RESULTS["pyevtx_rs_json"]:-N/A} | No support | No support |"
    } | tee "$out"

    echo ""
    echo "README-ready table saved to: $out"
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
    benchmark_pyevtx_rs
    benchmark_python_cpython
    benchmark_python_pypy

    # Optional head-to-head comparison (extra runtime)
    if [[ "$RUN_COMPARISON" == "1" ]]; then
        run_comparison_benchmark
    fi

    write_readme_table

    print_summary
}

main "$@"
