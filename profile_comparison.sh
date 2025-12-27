#!/usr/bin/env bash
#
# profile_comparison.sh - Compare Rust vs Zig EVTX parser performance
#
# See PERF.md for the hypothesis-driven workflow (before/after binaries, hyperfine JSON,
# samply profiles) and how to interpret allocator churn vs the Zig implementation.
#
# Usage:
#   ./profile_comparison.sh                    # Build + benchmark (no profiling)
#   ./profile_comparison.sh --bench-only       # Skip builds, just benchmark
#   ./profile_comparison.sh --profile-only     # Skip builds, just profile (opens samply UI)
#   ./profile_comparison.sh --top-leaves       # Profile both and print top leaf functions
#   ./profile_comparison.sh --flamegraph       # Generate flamegraphs (requires sudo on macOS)
#
# Environment variables:
#   SAMPLE_FILE     - EVTX file to use (default: samples/security_big_sample.evtx)
#   RUNS            - Number of hyperfine runs (default: 5)
#   OUTPUT_DIR      - Directory for results (default: ./profile_results)
#   ZIG_BINARY      - Path to Zig binary (default: ~/Workspace/zig-evtx/zig-out/bin/evtx_dump_zig)
#   TOP_LEAVES_N    - Number of leaf functions to print (default: 20)
#   QUIET_CHECK     - If set (e.g. 1), wait for a quiet system before profiling and use
#                    `hyperfine --prepare ./scripts/ensure_quiet.sh` for benchmarks.
#                    Tune thresholds via QUIET_* env vars (see `scripts/ensure_quiet.sh`).
#   BENCH_MT        - If set to 0, skip the 8-thread benchmark comparison (default: 1).
#                    Single-thread is the primary KPI for allocator-churn work, but 8T is still
#                    useful for end-to-end throughput comparisons.
#

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${SAMPLE_FILE:=$SCRIPT_DIR/samples/security_big_sample.evtx}"
: "${RUNS:=5}"
: "${OUTPUT_DIR:=$SCRIPT_DIR/profile_results}"
: "${ZIG_BINARY:=$HOME/Workspace/zig-evtx/zig-out/bin/evtx_dump_zig}"
: "${ZIG_PROJECT:=$HOME/Workspace/zig-evtx}"
: "${TOP_LEAVES_N:=20}"
: "${TOP_LEAVES_WEIGHT:=cpu}" # cpu | samples | wall

RUST_BINARY="$SCRIPT_DIR/target/release/evtx_dump"

QUIET_SCRIPT="$SCRIPT_DIR/scripts/ensure_quiet.sh"
QUIET_CHECK="${QUIET_CHECK:-0}"
BENCH_MT="${BENCH_MT:-1}"
HYPERFINE_PREPARE_ARGS=()
if [[ "$QUIET_CHECK" != "0" ]]; then
    if [[ ! -f "$QUIET_SCRIPT" ]]; then
        echo -e "${RED}Error: QUIET_CHECK is set but missing: $QUIET_SCRIPT${NC}"
        exit 1
    fi
    # `hyperfine --prepare` runs outside the measured timings; it’s ideal for waiting for quiet.
    HYPERFINE_PREPARE_ARGS=(--prepare "$QUIET_SCRIPT")
fi

maybe_wait_for_quiet() {
    if [[ "$QUIET_CHECK" != "0" ]]; then
        "$QUIET_SCRIPT"
    fi
}

print_top_leaves_table() {
    local profile_json="$1"
    local label="$2"
    local syms_json="${profile_json%.json}.syms.json"

    if [[ ! -f "$profile_json" ]]; then
        echo -e "${RED}Error: missing profile: $profile_json${NC}"
        return 1
    fi
    if [[ ! -f "$syms_json" ]]; then
        echo -e "${RED}Error: missing symbols sidecar: $syms_json${NC}"
        echo "Re-record with samply using --unstable-presymbolicate."
        return 1
    fi

    echo ""
    echo -e "${BLUE}=== Top leaf functions (${label}, ${TOP_LEAVES_WEIGHT}) ===${NC}"

    python3 - "$profile_json" "$syms_json" "$TOP_LEAVES_N" "$TOP_LEAVES_WEIGHT" <<'PY'
import bisect
import json
import sys

profile_path = sys.argv[1]
syms_path = sys.argv[2]
top_n = int(sys.argv[3])
weight_mode = (sys.argv[4] if len(sys.argv) > 4 else "cpu").strip().lower()

def load_json(path: str):
    with open(path, "rb") as f:
        data = f.read()
    if path.endswith(".gz"):
        import gzip
        data = gzip.decompress(data)
    return json.loads(data)

profile = load_json(profile_path)
syms = load_json(syms_path)

string_table = syms.get("string_table") or []
syms_data = syms.get("data") or []

def norm_hex(s: str) -> str:
    return s.upper()

syms_by_code = {}
syms_by_name = {}
preprocessed = {}

for entry in syms_data:
    code_id = entry.get("code_id")
    if isinstance(code_id, str) and code_id:
        syms_by_code[norm_hex(code_id)] = entry
        syms_by_code[norm_hex(code_id) + "0"] = entry  # common breakpad form
    debug_name = entry.get("debug_name")
    if isinstance(debug_name, str) and debug_name:
        syms_by_name[debug_name] = entry

    st = entry.get("symbol_table") or []
    st_sorted = sorted(st, key=lambda x: int(x.get("rva", 0)))
    rvas = [int(x.get("rva", 0)) for x in st_sorted]
    ends = [int(x.get("rva", 0)) + int(x.get("size", 0)) for x in st_sorted]
    names = []
    for x in st_sorted:
        si = x.get("symbol", 0)
        if isinstance(si, int) and 0 <= si < len(string_table):
            names.append(string_table[si])
        else:
            names.append("UNKNOWN")
    preprocessed[id(entry)] = (rvas, ends, names)

libs = profile.get("libs") or []

def match_entry_for_lib(lib: dict):
    for key in (lib.get("codeId"), lib.get("breakpadId")):
        if isinstance(key, str) and key:
            k = norm_hex(key)
            if k in syms_by_code:
                return syms_by_code[k]
            if k.endswith("0") and k[:-1] in syms_by_code:
                return syms_by_code[k[:-1]]
    for key in (lib.get("debugName"), lib.get("name")):
        if isinstance(key, str) and key and key in syms_by_name:
            return syms_by_name[key]
    return None

lib_entries = [match_entry_for_lib(lib) for lib in libs]

def lookup_symbol(lib_index: int | None, rva: int | None) -> str:
    if lib_index is None or rva is None:
        return "UNKNOWN"
    lib = libs[lib_index] if 0 <= lib_index < len(libs) else {}
    entry = lib_entries[lib_index] if 0 <= lib_index < len(lib_entries) else None
    if entry is None:
        name = lib.get("debugName") or lib.get("name") or f"lib{lib_index}"
        return f"{name} @ 0x{int(rva):x}"

    rvas, ends, names = preprocessed[id(entry)]
    i = bisect.bisect_right(rvas, int(rva)) - 1
    if i >= 0 and int(rva) < ends[i]:
        return names[i]

    name = lib.get("debugName") or lib.get("name") or entry.get("debug_name") or f"lib{lib_index}"
    return f"{name} @ 0x{int(rva):x}"

counts: dict[str, int] = {}
total = 0

for thread in (profile.get("threads") or []):
    samples = thread.get("samples") or {}
    stacks = samples.get("stack") or []
    sample_weights = samples.get("weight")
    cpu_deltas = samples.get("threadCPUDelta")
    wall_deltas = samples.get("timeDeltas")

    stack_table = thread.get("stackTable") or {}
    frame_table = thread.get("frameTable") or {}
    func_table = thread.get("funcTable") or {}
    resource_table = thread.get("resourceTable") or {}

    stack_frame = stack_table.get("frame") or []
    frame_addr = frame_table.get("address") or []
    frame_func = frame_table.get("func") or []
    func_resource = func_table.get("resource") or []
    resource_lib = resource_table.get("lib") or []

    for idx, stack_id in enumerate(stacks):
        if not isinstance(stack_id, int):
            continue
        if stack_id < 0 or stack_id >= len(stack_frame):
            continue
        frame_id = stack_frame[stack_id]
        if not isinstance(frame_id, int):
            continue
        if frame_id < 0 or frame_id >= len(frame_addr) or frame_id >= len(frame_func):
            continue

        rva = frame_addr[frame_id]
        func_id = frame_func[frame_id]

        lib_index = None
        if isinstance(func_id, int) and 0 <= func_id < len(func_resource):
            resource_id = func_resource[func_id]
            if isinstance(resource_id, int) and 0 <= resource_id < len(resource_lib):
                lib_index = resource_lib[resource_id]

        w = 1
        if weight_mode == "cpu" and isinstance(cpu_deltas, list) and idx < len(cpu_deltas):
            try:
                w = int(cpu_deltas[idx])
            except Exception:
                w = 0
        elif weight_mode == "wall" and isinstance(wall_deltas, list) and idx < len(wall_deltas):
            # timeDeltas is in ms (float). Keep as ms*1000 integer so output formatting is consistent.
            try:
                w = int(float(wall_deltas[idx]) * 1000.0)
            except Exception:
                w = 0
        elif isinstance(sample_weights, list) and idx < len(sample_weights):
            try:
                w = int(sample_weights[idx])
            except Exception:
                w = 1

        total += w
        leaf = lookup_symbol(lib_index, rva)
        counts[leaf] = counts.get(leaf, 0) + w

items = sorted(counts.items(), key=lambda kv: kv[1], reverse=True)[:top_n]

if weight_mode == "cpu":
    header = "CPU ms"
    divisor = 1000.0  # µs -> ms
elif weight_mode == "wall":
    header = "Wall ms"
    divisor = 1000.0  # (ms*1000) -> ms
else:
    header = "Samples"
    divisor = 1.0

print(f"| # | {header} | % | Leaf |")
print("| -: | --: | --: | --- |")
for i, (name, count) in enumerate(items, start=1):
    pct = (count / total * 100.0) if total else 0.0
    v = count / divisor
    if divisor == 1.0:
        v_str = str(int(v))
    else:
        v_str = f"{v:,.1f}"
    print(f"| {i} | {v_str} | {pct:5.1f}% | {name} |")
PY
}

# Parse arguments
BUILD=true
BENCH=true
PROFILE=false
FLAMEGRAPH=false
TOP_LEAVES=false

for arg in "$@"; do
    case $arg in
        --bench-only)
            BUILD=false
            PROFILE=false
            ;;
        --profile-only)
            BUILD=false
            BENCH=false
            PROFILE=true
            ;;
        --top-leaves)
            BENCH=false
            PROFILE=true
            TOP_LEAVES=true
            ;;
        --flamegraph)
            BUILD=false
            BENCH=false
            FLAMEGRAPH=true
            ;;
        --help|-h)
            head -20 "$0" | tail -18
            exit 0
            ;;
    esac
done

echo -e "${BLUE}=== EVTX Parser Performance Comparison ===${NC}"
echo ""

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Validate sample file exists
if [[ ! -f "$SAMPLE_FILE" ]]; then
    echo -e "${RED}Error: Sample file not found: $SAMPLE_FILE${NC}"
    exit 1
fi

SAMPLE_SIZE=$(ls -lh "$SAMPLE_FILE" | awk '{print $5}')
echo -e "Sample file: ${GREEN}$SAMPLE_FILE${NC} ($SAMPLE_SIZE)"
echo ""

# Build phase
if [[ "$BUILD" == true ]]; then
    echo -e "${YELLOW}Building Rust (release + fast-alloc)...${NC}"
    (cd "$SCRIPT_DIR" && cargo build --release --features fast-alloc 2>&1 | tail -3)

    if [[ -d "$ZIG_PROJECT" ]]; then
        echo -e "${YELLOW}Building Zig (ReleaseFast)...${NC}"
        (cd "$ZIG_PROJECT" && zig build -Doptimize=ReleaseFast 2>&1 | tail -3) || echo "Zig build skipped"
    fi
    echo ""
fi

# Validate binaries exist
if [[ ! -x "$RUST_BINARY" ]]; then
    echo -e "${RED}Error: Rust binary not found: $RUST_BINARY${NC}"
    echo "Run: cargo build --release --features fast-alloc"
    exit 1
fi

if [[ ! -x "$ZIG_BINARY" ]]; then
    echo -e "${RED}Error: Zig binary not found: $ZIG_BINARY${NC}"
    echo "Run: cd ~/Workspace/zig-evtx && zig build -Doptimize=ReleaseFast"
    exit 1
fi

# Benchmark phase
if [[ "$BENCH" == true ]]; then
    echo -e "${YELLOW}Running benchmarks (${RUNS} runs each)...${NC}"
    echo ""

    TIMESTAMP=$(date +%Y%m%d_%H%M%S)
    BENCH_FILE="$OUTPUT_DIR/benchmark_${TIMESTAMP}.md"

    hyperfine \
        "${HYPERFINE_PREPARE_ARGS[@]}" \
        --warmup 2 \
        --runs "$RUNS" \
        --export-markdown "$BENCH_FILE" \
        --export-json "$OUTPUT_DIR/benchmark_${TIMESTAMP}.json" \
        -n "Rust (fast-alloc)" "$RUST_BINARY -t 1 -o jsonl $SAMPLE_FILE" \
        -n "Zig" "$ZIG_BINARY -t 1 --no-checks -o jsonl $SAMPLE_FILE" \
        2>&1

    echo ""
    echo -e "${GREEN}Benchmark results saved to: $BENCH_FILE${NC}"

    # Optional: multi-threaded comparison (on by default).
    if [[ "${BENCH_MT}" != "0" ]]; then
        echo ""
        echo -e "${YELLOW}Running multi-threaded comparison (8 threads)...${NC}"

        hyperfine \
            "${HYPERFINE_PREPARE_ARGS[@]}" \
            --warmup 2 \
            --runs "$RUNS" \
            --export-markdown "$OUTPUT_DIR/benchmark_mt_${TIMESTAMP}.md" \
            -n "Rust 8T" "$RUST_BINARY -t 8 -o jsonl $SAMPLE_FILE" \
            -n "Zig 8T" "$ZIG_BINARY -t 8 --no-checks -o jsonl $SAMPLE_FILE" \
            2>&1 || echo "Multi-threaded benchmark failed (may need --features multithreading)"

        echo ""
    fi
fi

# Profile phase (samply - opens browser UI)
if [[ "$PROFILE" == true ]]; then
    echo -e "${YELLOW}Profiling with samply...${NC}"
    echo ""

    if ! command -v samply &> /dev/null; then
        echo -e "${RED}samply not found. Install with: cargo install samply${NC}"
        exit 1
    fi

    if [[ "$TOP_LEAVES" == true ]]; then
        choice=3
    else
        echo -e "${BLUE}Choose what to profile:${NC}"
        echo "  1) Rust only"
        echo "  2) Zig only"
        echo "  3) Both (Rust first, then Zig)"
        read -p "Selection [1-3]: " choice
    fi

    case $choice in
        1)
            echo -e "${YELLOW}Profiling Rust...${NC}"
            # Save profile + sidecar symbols file so `samply load` shows function names.
            maybe_wait_for_quiet
            samply record --unstable-presymbolicate -o "$OUTPUT_DIR/rust_profile.json" -- \
                "$RUST_BINARY" -t 1 -o jsonl "$SAMPLE_FILE"
            ;;
        2)
            echo -e "${YELLOW}Profiling Zig...${NC}"
            # Save profile + sidecar symbols file so `samply load` shows function names.
            maybe_wait_for_quiet
            samply record --unstable-presymbolicate -o "$OUTPUT_DIR/zig_profile.json" -- \
                "$ZIG_BINARY" -t 1 --no-checks -o jsonl "$SAMPLE_FILE"
            ;;
        3)
            echo -e "${YELLOW}Recording Rust profile...${NC}"
            maybe_wait_for_quiet
            samply record --save-only --unstable-presymbolicate -o "$OUTPUT_DIR/rust_profile.json" -- \
                "$RUST_BINARY" -t 1 -o jsonl "$SAMPLE_FILE" > /dev/null 2>&1

            echo -e "${YELLOW}Recording Zig profile...${NC}"
            maybe_wait_for_quiet
            samply record --save-only --unstable-presymbolicate -o "$OUTPUT_DIR/zig_profile.json" -- \
                "$ZIG_BINARY" -t 1 --no-checks -o jsonl "$SAMPLE_FILE" > /dev/null 2>&1

            echo ""
            echo -e "${GREEN}Profiles saved:${NC}"
            echo "  Rust: $OUTPUT_DIR/rust_profile.json"
            echo "        $OUTPUT_DIR/rust_profile.syms.json"
            echo "  Zig:  $OUTPUT_DIR/zig_profile.json"
            echo "        $OUTPUT_DIR/zig_profile.syms.json"
            echo ""
            echo "View with:"
            echo "  samply load $OUTPUT_DIR/rust_profile.json"
            echo "  samply load $OUTPUT_DIR/zig_profile.json"

            if [[ "$TOP_LEAVES" == true ]]; then
                print_top_leaves_table "$OUTPUT_DIR/rust_profile.json" "Rust"
                print_top_leaves_table "$OUTPUT_DIR/zig_profile.json" "Zig"
            fi
            ;;
    esac
fi

# Flamegraph phase (cargo-flamegraph - may need sudo on macOS)
if [[ "$FLAMEGRAPH" == true ]]; then
    echo -e "${YELLOW}Generating flamegraphs...${NC}"
    echo -e "${RED}Note: This may require sudo on macOS${NC}"
    echo ""

    if ! command -v cargo-flamegraph &> /dev/null && ! command -v flamegraph &> /dev/null; then
        echo -e "${RED}flamegraph not found. Install with: cargo install flamegraph${NC}"
        exit 1
    fi

    TIMESTAMP=$(date +%Y%m%d_%H%M%S)

    # Rust flamegraph
    echo -e "${YELLOW}Generating Rust flamegraph...${NC}"
    (cd "$SCRIPT_DIR" && cargo flamegraph \
        --root \
        --bin evtx_dump \
        --features fast-alloc \
        --output "$OUTPUT_DIR/flamegraph_rust_${TIMESTAMP}.svg" \
        -- -t 1 -o jsonl "$SAMPLE_FILE" > /dev/null 2>&1) || {
        echo -e "${RED}Rust flamegraph failed (may need sudo)${NC}"
    }

    # For Zig, use dtrace directly or samply
    echo -e "${YELLOW}Generating Zig flamegraph via samply...${NC}"
    maybe_wait_for_quiet
    samply record --save-only --unstable-presymbolicate -o "$OUTPUT_DIR/zig_profile_${TIMESTAMP}.json" \
        -- "$ZIG_BINARY" -t 1 --no-checks -o jsonl "$SAMPLE_FILE" > /dev/null 2>&1

    echo ""
    echo -e "${GREEN}Flamegraphs saved to: $OUTPUT_DIR/${NC}"
    ls -la "$OUTPUT_DIR"/*.svg 2>/dev/null || echo "(SVG files may require sudo)"
fi

# Summary
echo ""
echo -e "${BLUE}=== Quick Commands ===${NC}"
echo ""
echo "# Benchmark only:"
echo "  ./profile_comparison.sh --bench-only"
echo ""
echo "# Benchmark (wait for quiet machine via scripts/ensure_quiet.sh):"
echo "  QUIET_CHECK=1 ./profile_comparison.sh --bench-only"
echo ""
echo "# Benchmark without multi-thread comparison:"
echo "  BENCH_MT=0 ./profile_comparison.sh --bench-only"
echo ""
echo "# Interactive profiling (opens browser):"
echo "  ./profile_comparison.sh --profile-only"
echo ""
echo "# View saved profiles:"
echo "  samply load $OUTPUT_DIR/rust_profile.json"
echo "  samply load $OUTPUT_DIR/zig_profile.json"
echo ""
echo "# Generate flamegraphs (may need sudo):"
echo "  ./profile_comparison.sh --flamegraph"
echo ""
