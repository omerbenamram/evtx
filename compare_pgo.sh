#!/bin/bash
set -euo pipefail

# Display colorful output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Define benchmark parameters
WARMUP_RUNS=3
BENCHMARK_RUNS=10
OUTPUT_DIR="benchmark_results/pgo_comparison"

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Check for hyperfine
echo -e "${BLUE}Checking dependencies...${NC}"
if ! command -v hyperfine &>/dev/null; then
    echo -e "${BLUE}Installing hyperfine...${NC}"
    brew install hyperfine
else
    echo -e "${GREEN}Hyperfine already installed.${NC}"
fi

# Function to determine the default target
get_default_target() {
    case "$(uname -sm)" in
    "Darwin x86_64") echo "x86_64-apple-darwin" ;;
    "Darwin arm64") echo "aarch64-apple-darwin" ;;
    "Linux x86_64") echo "x86_64-unknown-linux-gnu" ;;
    "Linux aarch64") echo "aarch64-unknown-linux-gnu" ;;
    *) echo "unknown" ;;
    esac
}

# Use the provided TARGET or default to the current machine's target
TARGET=${TARGET:-$(get_default_target)}

if [ "$TARGET" = "unknown" ]; then
    echo -e "${RED}Error: Unable to determine default target. Please specify TARGET explicitly.${NC}"
    exit 1
fi

echo -e "${BLUE}Using target: $TARGET${NC}"

# Find the largest sample file to benchmark with
echo -e "${BLUE}Finding largest EVTX sample file...${NC}"
SAMPLE_FILE=$(find samples -name "*.evtx" -type f -exec du -k {} \; | sort -nr | head -1 | cut -f2)

if [ -z "$SAMPLE_FILE" ]; then
    echo -e "${RED}No sample EVTX files found. Please place at least one .evtx file in the samples directory.${NC}"
    exit 1
fi

# Get file size in KB for display
FILE_SIZE_KB=$(du -k "$SAMPLE_FILE" | cut -f1)
echo -e "${GREEN}Using largest sample file: $SAMPLE_FILE (${FILE_SIZE_KB} KB)${NC}"

# Build PGO version
echo -e "${BLUE}Building PGO-optimized version...${NC}"
./build_pgo.sh
cp "target/$TARGET/release/evtx_dump" "target/$TARGET/release/evtx_dump_pgo"

# Build regular version
echo -e "${BLUE}Building regular release version...${NC}"
cargo build --release --bin evtx_dump --target $TARGET --features fast-alloc
cp "target/$TARGET/release/evtx_dump" "target/$TARGET/release/evtx_dump_regular"

# Run single-threaded benchmarks
echo -e "\n${YELLOW}Running single-threaded benchmarks (-t 1)...${NC}"
hyperfine --warmup $WARMUP_RUNS \
    --export-markdown "$OUTPUT_DIR/single_thread_benchmarks.md" \
    --export-json "$OUTPUT_DIR/single_thread_benchmarks.json" \
    --export-csv "$OUTPUT_DIR/single_thread_benchmarks.csv" \
    --setup "sync && sudo purge" \
    --prepare "sleep 1" \
    --runs $BENCHMARK_RUNS \
    --command-name "PGO" "target/$TARGET/release/evtx_dump_pgo -t 1 -o json $SAMPLE_FILE > /dev/null" \
    --command-name "Regular" "target/$TARGET/release/evtx_dump_regular -t 1 -o json $SAMPLE_FILE > /dev/null"

# Run multi-threaded benchmarks
echo -e "\n${YELLOW}Running multi-threaded benchmarks (-t 8)...${NC}"
hyperfine --warmup $WARMUP_RUNS \
    --export-markdown "$OUTPUT_DIR/multi_thread_benchmarks.md" \
    --export-json "$OUTPUT_DIR/multi_thread_benchmarks.json" \
    --export-csv "$OUTPUT_DIR/multi_thread_benchmarks.csv" \
    --setup "sync && sudo purge" \
    --prepare "sleep 1" \
    --runs $BENCHMARK_RUNS \
    --command-name "PGO" "target/$TARGET/release/evtx_dump_pgo -t 8 -o json $SAMPLE_FILE > /dev/null" \
    --command-name "Regular" "target/$TARGET/release/evtx_dump_regular -t 8 -o json $SAMPLE_FILE > /dev/null"

# Display results
if [ $? -eq 0 ]; then
    echo -e "\n${GREEN}Benchmarks complete!${NC}"

    echo -e "\n${YELLOW}Single-threaded (-t 1) results:${NC}"
    cat "$OUTPUT_DIR/single_thread_benchmarks.md"

    echo -e "\n${YELLOW}Multi-threaded (-t 8) results:${NC}"
    cat "$OUTPUT_DIR/multi_thread_benchmarks.md"

    echo -e "\n${GREEN}Detailed results saved to:${NC}"
    echo -e "  - ${BLUE}$OUTPUT_DIR/${NC}"
else
    echo -e "${RED}Benchmark failed.${NC}"
    exit 1
fi
