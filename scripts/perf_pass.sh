#!/usr/bin/env bash
# Measures one stage of a performance pass: saves the release binary, benchmarks it with
# hyperfine (the post stage against the pre binary), and records a samply profile.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'EOF'
Usage: scripts/perf_pass.sh <tag> <pre|post> [options] [-- <evtx_dump arguments>]

Run it with `pre` before the change and with `post` after it, with the same tag and options.

Options:
  --file <path>     EVTX input file (default: samples/security_big_sample.evtx)
  --cachegrind      Also count instructions with valgrind, for a difference within noise.
                    The post stage counts both binaries, so it is enough to pass it there.
  --skip-samply     Do not record a profile
  -h, --help        Show this help

The evtx_dump arguments default to `-t 1 -o json`.

Output: binaries/evtx_dump_<tag>_<stage>, benchmarks/*_<tag>_<stage>.txt and
profile/*<tag>_<stage>* (all three folders are in .gitignore).
EOF
}

if [[ $# -gt 0 && ( "$1" == "-h" || "$1" == "--help" ) ]]; then
  usage
  exit 0
fi
if [[ $# -lt 2 ]]; then
  usage >&2
  exit 1
fi

TAG="$1"
STAGE="$2"
shift 2
if [[ "$STAGE" != "pre" && "$STAGE" != "post" ]]; then
  echo "The stage must be 'pre' or 'post', not '$STAGE'." >&2
  exit 1
fi

FILE="$ROOT/samples/security_big_sample.evtx"
CACHEGRIND=0
SKIP_SAMPLY=0
DUMP_ARGS=(-t 1 -o json)

while [[ $# -gt 0 ]]; do
  case "$1" in
    --file)
      [[ $# -ge 2 ]] || { echo "--file needs a path." >&2; exit 1; }
      # Relative to where the script was called from, not to the repository.
      FILE="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
      shift 2;;
    --cachegrind) CACHEGRIND=1; shift;;
    --skip-samply) SKIP_SAMPLY=1; shift;;
    -h|--help) usage; exit 0;;
    --) shift; DUMP_ARGS=("$@"); break;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 1;;
  esac
done

cd "$ROOT"
BIN="binaries/evtx_dump_${TAG}_${STAGE}"
PRE_BIN="binaries/evtx_dump_${TAG}_pre"

# Before the build, so that a wrong tag does not cost a release build.
if [[ "$STAGE" == "pre" && -e "$BIN" ]]; then
  echo "$BIN exists. A second 'pre' run would replace the baseline: use a new tag, or delete it." >&2
  exit 1
fi
if [[ "$STAGE" == "post" && ! -x "$PRE_BIN" ]]; then
  echo "$PRE_BIN is missing. Run the 'pre' stage with this tag before the change." >&2
  exit 1
fi

if [[ ! -f "$FILE" ]]; then
  echo "$FILE is not a file." >&2
  exit 1
fi
if ! command -v hyperfine >/dev/null; then
  echo "hyperfine is not installed." >&2
  exit 1
fi

mkdir -p binaries benchmarks profile

# The same build as the release: `fast-alloc` and nothing else.
cargo build --release --features fast-alloc --bin evtx_dump
cp target/release/evtx_dump "$BIN"

# The post stage measures both binaries.
BINS=("$PRE_BIN")
if [[ "$STAGE" == "post" ]]; then
  BINS+=("$BIN")
fi

# hyperfine takes each command as one string and discards its output.
COMMANDS=()
for bin in "${BINS[@]}"; do
  printf -v command '%q ' "$bin" ${DUMP_ARGS[@]+"${DUMP_ARGS[@]}"} "$FILE"
  COMMANDS+=("$command")
done
hyperfine -w 10 -r 20 "${COMMANDS[@]}" | tee "benchmarks/benchmark_${TAG}_${STAGE}.txt"

if [[ $CACHEGRIND -eq 1 ]]; then
  for bin in "${BINS[@]}"; do
    valgrind --tool=cachegrind --cache-sim=no --cachegrind-out-file=/dev/null \
      "$bin" ${DUMP_ARGS[@]+"${DUMP_ARGS[@]}"} "$FILE" 2>&1 >/dev/null
  done | tee "benchmarks/cachegrind_${TAG}_${STAGE}.txt"
fi

if [[ $SKIP_SAMPLY -eq 0 ]]; then
  # The loop binary parses the file many times in one process, which gives samply enough
  # samples. It always renders JSON with its default settings, without indentation, on the
  # system allocator: the profile follows `--file`, and not the evtx_dump arguments.
  cargo build --release --features "fast-alloc bench" --bin bench_evtx_dump_loop
  PROFILE="profile/${TAG}_${STAGE}.json.gz"
  if samply record --save-only --unstable-presymbolicate -o "$PROFILE" -- \
      target/release/bench_evtx_dump_loop --file "$FILE" --loops 20 --threads 1; then
    python3 scripts/samply_extract_tables.py \
      --profile "$PROFILE" \
      --syms "${PROFILE%.gz}.syms.json" \
      --out-dir profile --label "${TAG}_${STAGE}"
  else
    echo "samply did not record a profile: see its message above (on Linux it needs perf events). The benchmark stands." >&2
  fi
fi
