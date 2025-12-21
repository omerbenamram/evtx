#!/usr/bin/env bash
set -euo pipefail

say() {
  printf '%s\n' "$*"
}

if command -v cargo >/dev/null 2>&1; then
  say "[worktree] Prefetching Rust deps (root crate)..."
  cargo fetch --locked --features fast-alloc || cargo fetch --locked || cargo fetch

  if [[ -f "evtx-wasm/Cargo.toml" ]]; then
    say "[worktree] Prefetching Rust deps (evtx-wasm)..."
    (cd evtx-wasm && (cargo fetch --locked || cargo fetch))
  fi
else
  say "[worktree] cargo not found; skipping Rust dependency prefetch."
fi

VIEWER_DIR="evtx-wasm/evtx-viewer"
SKIP_VIEWER_SETUP="${CURSOR_SKIP_VIEWER_SETUP:-}"
if [[ -f ".cursor/skip-viewer-setup" ]]; then
  SKIP_VIEWER_SETUP="1"
fi

if [[ -n "${SKIP_VIEWER_SETUP}" ]]; then
  say "[worktree] Skipping viewer deps install (CURSOR_SKIP_VIEWER_SETUP or .cursor/skip-viewer-setup present)."
elif [[ -f "${VIEWER_DIR}/package.json" ]]; then
  if command -v bun >/dev/null 2>&1; then
    say "[worktree] Installing viewer deps with bun (${VIEWER_DIR})..."
    (cd "${VIEWER_DIR}" && (bun install --frozen-lockfile || bun install))
  else
    say "[worktree] bun not found; skipping viewer deps install (${VIEWER_DIR})."
  fi
fi

say "[worktree] Setup complete."


