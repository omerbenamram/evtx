#!/usr/bin/env bash
# Build the evtx-wasm crate for the browser target and start the viewer dev server.
#
# Usage:
#   ./run_viewer.sh            # builds in release mode (optimised) and starts Vite
#   ./run_viewer.sh --debug    # builds with debug symbols for easier tracing
#
# NOTE: Requires `wasm-pack`, `bun` (or npm/yarn) and Vite installed.

set -euo pipefail

# -------- settings ----------
CRATE_DIR="evtx-wasm"                  # Rust crate path (relative to repo root)
VIEWER_DIR="evtx-wasm/evtx-viewer"     # React viewer path
VIEWER_WASM_DIR="$VIEWER_DIR/src/wasm" # Where the TS code expects the generated bindings
OUT_NAME="evtx_wasm"                   # Base filename for the generated JS/WASM artefacts
# ----------------------------

MODE="release"
if [[ ${1:-} == "--debug" || ${1:-} == "-d" ]]; then
    MODE="debug"
    shift
fi

echo "📦 Building WASM in $MODE mode..."

# Clean previous artefacts so we never serve stale code
rm -rf "$VIEWER_WASM_DIR"
mkdir -p "$VIEWER_WASM_DIR"

# Convert to an absolute path so it stays valid after we cd into the crate dir
ABS_VIEWER_WASM_DIR="$(cd "$(dirname "$VIEWER_WASM_DIR")" && pwd)/$(basename "$VIEWER_WASM_DIR")"

pushd "$CRATE_DIR" >/dev/null

BUILD_FLAGS=(
    --target web                     # generate browser-compatible bindings
    --out-dir "$ABS_VIEWER_WASM_DIR" # place them where the viewer imports from
    --out-name "$OUT_NAME"           # keep filenames stable
)

if [[ "$MODE" == "debug" ]]; then
    wasm-pack build "${BUILD_FLAGS[@]}" --debug
else
    wasm-pack build "${BUILD_FLAGS[@]}" --release
fi

popd >/dev/null

echo "✅ WASM build complete – artefacts are in $ABS_VIEWER_WASM_DIR"

echo "🚀 Starting Vite dev server... (press Ctrl+C to stop)"

pushd "$VIEWER_DIR" >/dev/null

# Ensure JS/TS deps are installed (cheap if already up-to-date)
bun install

# Forward any leftover CLI args to the dev server (e.g. --open, --host)
bun run dev "$@"

popd >/dev/null
