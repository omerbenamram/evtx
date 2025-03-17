#!/bin/bash

# Set up colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}===== MIMALLOC + PGO INTERACTION TEST =====${NC}"
echo "Started at $(date)"

# Add the binary to Cargo.toml if needed
if ! grep -q "mimalloc_pgo_test" /Users/omerba/Workspace/evtx/Cargo.toml; then
    echo -e "${BLUE}Adding test binary to Cargo.toml${NC}"
    echo '
[[bin]]
name = "mimalloc_pgo_test"
path = "src/bin/mimalloc_pgo_test.rs"' >>/Users/omerba/Workspace/evtx/Cargo.toml
fi

# Clean up previous profile data
echo -e "${BLUE}Cleaning up previous profile data${NC}"
rm -rf /tmp/pgo-data
rm -rf /tmp/mimalloc_pgo_output.txt
rm -rf /tmp/mimalloc_pgo_debug.log
mkdir -p /tmp/pgo-data
chmod 777 /tmp/pgo-data

# Build the test binary with profiling and mimalloc
echo -e "${BLUE}Building test binary with PGO instrumentation and mimalloc${NC}"
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data -g" cargo +nightly build --release --bin mimalloc_pgo_test --features fast-alloc-mimalloc-secure

# Create LLDB script for debugging
echo -e "${BLUE}Creating LLDB script${NC}"
cat >/tmp/mimalloc_pgo.lldb <<EOF
# Set breakpoints
breakpoint set -n __llvm_profile_write_file
breakpoint set -n mi_malloc
breakpoint set -n mi_free

# Set commands for the profile write function
breakpoint command add -o "bt" -o "frame variable" 1

# Handle segfaults
process handle -p true -s true -n true SIGSEGV SIGBUS SIGABRT

# Run with environment variable
process launch --environment LLVM_PROFILE_FILE="/tmp/pgo-data/%p-%m.profraw"

# If we crash, get detailed info
bt all
register read --all
thread info
EOF

# Run normally first (without debugger) with environment variable
echo -e "${BLUE}Running normally first${NC}"
export LLVM_PROFILE_FILE="/tmp/pgo-data/%p-%m.profraw"
echo "LLVM_PROFILE_FILE=$LLVM_PROFILE_FILE"
./target/release/mimalloc_pgo_test >/tmp/mimalloc_pgo_output.txt 2>&1 || {
    echo -e "${RED}Program crashed with exit code $?${NC}"
    tail -n 20 /tmp/mimalloc_pgo_output.txt
}

# Check if any profile data was created
echo -e "${BLUE}Checking for profile data${NC}"
ls -la /tmp/pgo-data/

# Run with LLDB to get more info if it crashed
# echo -e "${BLUE}Running with LLDB${NC}"
# lldb -s /tmp/mimalloc_pgo.lldb ./target/release/mimalloc_pgo_test >/tmp/mimalloc_pgo_debug.log 2>&1

# Print key parts of the debug log
echo -e "${BLUE}Debug log highlights${NC}"
grep -A 10 "SIGSEGV\|EXC_BAD_ACCESS" /tmp/mimalloc_pgo_debug.log || echo "No crash signals found in debug log"
grep -A 20 -B 1 "frame #0:" /tmp/mimalloc_pgo_debug.log | head -20 || echo "No frames found in debug log"

echo -e "${BLUE}Last 20 lines of debug log${NC}"
tail -n 20 /tmp/mimalloc_pgo_debug.log

echo -e "${GREEN}Test complete${NC}"
echo "Finished at $(date)"
