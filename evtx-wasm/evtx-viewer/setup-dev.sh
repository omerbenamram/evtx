#!/bin/bash

# Create symlink to WASM files for development
mkdir -p public/pkg

# Get absolute paths
PARENT_PKG_DIR="$(cd ../public/pkg && pwd)"
CURRENT_PKG_DIR="$(pwd)/public/pkg"

# Create symlinks with absolute paths
ln -sf "$PARENT_PKG_DIR/evtx_wasm_bg.wasm" "$CURRENT_PKG_DIR/evtx_wasm_bg.wasm"
ln -sf "$PARENT_PKG_DIR/evtx_wasm.js" "$CURRENT_PKG_DIR/evtx_wasm.js"
ln -sf "$PARENT_PKG_DIR/evtx_wasm.d.ts" "$CURRENT_PKG_DIR/evtx_wasm.d.ts"

echo "✅ Development environment set up. WASM files linked."
echo "   Source: $PARENT_PKG_DIR"
echo "   Target: $CURRENT_PKG_DIR"