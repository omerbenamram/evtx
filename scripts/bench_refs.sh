#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   scripts/bench_refs.sh <clean_ref> <mod_ref> [evtx_file]
# Example:
#   scripts/bench_refs.sh HEAD~1 HEAD

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CLEAN_REF="${1:-}"
MOD_REF="${2:-}"
EVTX_FILE="${3:-$REPO_DIR/samples/security_big_sample.evtx}"

if [[ -z "$CLEAN_REF" || -z "$MOD_REF" ]]; then
  echo "Usage: $0 <clean_ref> <mod_ref> [evtx_file]" >&2
  exit 1
fi

mkdir -p "$REPO_DIR/binaries" "$REPO_DIR/benchmarks" "$REPO_DIR/tmp/worktrees"
TS="$(date -u +%Y%m%dT%H%M%SZ)"

# Clean worktree
CWT="$REPO_DIR/tmp/worktrees/clean-${CLEAN_REF//\//-}-$TS"
git worktree add --force --detach "$CWT" "$CLEAN_REF" >/dev/null
( cd "$CWT" && cargo build --release --features fast-alloc >/dev/null )
CLEAN_HASH="$(git -C "$CWT" rev-parse --short HEAD)"
CLEAN_BIN="$REPO_DIR/binaries/evtx_dump_${CLEAN_HASH}_${TS}_clean"
cp "$CWT/target/release/evtx_dump" "$CLEAN_BIN"

# Mod worktree
MWT="$REPO_DIR/tmp/worktrees/mod-${MOD_REF//\//-}-$TS"
git worktree add --force --detach "$MWT" "$MOD_REF" >/dev/null
( cd "$MWT" && cargo build --release --features fast-alloc >/dev/null )
MOD_HASH="$(git -C "$MWT" rev-parse --short HEAD)"
MOD_BIN="$REPO_DIR/binaries/evtx_dump_${MOD_HASH}_${TS}_mod"
cp "$MWT/target/release/evtx_dump" "$MOD_BIN"

# Benchmark pair
"$REPO_DIR/scripts/run_benchmark_pair.sh" "$CLEAN_BIN" "$MOD_BIN" "$EVTX_FILE"

# Cleanup worktrees
git worktree remove --force "$CWT" >/dev/null || true
git worktree remove --force "$MWT" >/dev/null || true
git worktree prune >/dev/null || true

echo "Clean: $CLEAN_BIN"
echo "Mod  : $MOD_BIN"


