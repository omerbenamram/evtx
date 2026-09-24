#!/usr/bin/env bash
# Build the WASM bindings and start the viewer. Usage: ./run_viewer.sh [--debug] [vite args]
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
profile=--release
if [[ ${1:-} == --debug || ${1:-} == -d ]]; then profile=--debug; shift; fi
rm -rf evtx-wasm/evtx-viewer/src/wasm
wasm-pack build evtx-wasm --target web --out-dir evtx-viewer/src/wasm --out-name evtx_wasm "$profile"
cd evtx-wasm/evtx-viewer
mkdir -p public/samples
ln -sf "$PWD/../../samples/security.evtx" public/samples/security.evtx
bun install --frozen-lockfile
exec bun run dev "$@"
