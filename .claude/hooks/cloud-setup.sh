#!/bin/bash
# Cloud (Claude Code on the web) session setup. No-op locally.
set -euo pipefail
[ "${CLAUDE_CODE_REMOTE:-}" = "true" ] || exit 0
cd "$CLAUDE_PROJECT_DIR"

# samples/dlls/* are LFS; WEVT_TEMPLATE tests read them.
git lfs install --local >/dev/null && git lfs pull

# edition 2024 needs a recent stable. rustfmt is for the format check, which CI runs.
rustup toolchain install stable --profile minimal --component rustfmt >/dev/null
cargo fetch --quiet
