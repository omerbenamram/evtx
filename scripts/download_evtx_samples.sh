#!/usr/bin/env bash
#
# Download sample EVTX files for benchmarking
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SAMPLES_DIR="${SAMPLES_DIR:-$ROOT/samples}"

mkdir -p "$SAMPLES_DIR"

echo "Downloading sample EVTX files to $SAMPLES_DIR..."
echo ""

# Option 1: EVTX-ATTACK-SAMPLES (many small files, good variety)
ATTACK_SAMPLES_DIR="$SAMPLES_DIR/EVTX-ATTACK-SAMPLES"
if [[ ! -d "$ATTACK_SAMPLES_DIR" ]]; then
  echo "Cloning EVTX-ATTACK-SAMPLES (many diverse samples)..."
  git clone --depth 1 https://github.com/sbousseaden/EVTX-ATTACK-SAMPLES.git "$ATTACK_SAMPLES_DIR"
  echo "Done!"
else
  echo "EVTX-ATTACK-SAMPLES already exists"
fi

# Count total size
TOTAL_SIZE=$(du -sh "$ATTACK_SAMPLES_DIR" 2>/dev/null | cut -f1)
EVTX_COUNT=$(find "$ATTACK_SAMPLES_DIR" -name "*.evtx" 2>/dev/null | wc -l | xargs)

echo ""
echo "=== Downloaded Samples ==="
echo "Location: $ATTACK_SAMPLES_DIR"
echo "Total size: $TOTAL_SIZE"
echo "EVTX files: $EVTX_COUNT"
echo ""

# Find largest file for benchmarking
echo "Largest EVTX files (best for benchmarking):"
find "$ATTACK_SAMPLES_DIR" -name "*.evtx" -exec du -h {} \; 2>/dev/null | sort -rh | head -10

# Create a combined file for bigger benchmarks
echo ""
echo "=== Creating Combined Test File ==="

COMBINED_FILE="$SAMPLES_DIR/combined_samples.evtx"
if [[ ! -f "$COMBINED_FILE" ]]; then
  # Just use the largest file we find
  LARGEST=$(find "$ATTACK_SAMPLES_DIR" -name "*.evtx" -exec du -b {} \; 2>/dev/null | sort -rn | head -1 | cut -f2)
  if [[ -n "$LARGEST" ]]; then
    cp "$LARGEST" "$COMBINED_FILE"
    echo "Copied largest file as: $COMBINED_FILE"
    echo "Size: $(du -h "$COMBINED_FILE" | cut -f1)"
  fi
else
  echo "Combined file already exists: $COMBINED_FILE ($(du -h "$COMBINED_FILE" | cut -f1))"
fi

echo ""
echo "=== Next Steps ==="
echo ""
echo "For quick benchmarks, use any .evtx file:"
echo "  ./scripts/bench_parsers.sh --file samples/EVTX-ATTACK-SAMPLES/some_file.evtx"
echo ""
echo "For comprehensive benchmarks, you may want a larger (~30MB+) Security.evtx file."
echo "These typically come from Windows machines with audit logging enabled."
echo ""
echo "To export from a Windows machine:"
echo "  wevtutil epl Security C:\\path\\to\\security.evtx"
echo ""
echo "Or download additional samples from:"
echo "  - https://github.com/NextronSystems/evtx-baseline"
echo "  - Your own Windows test environments"
