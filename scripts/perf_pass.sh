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
    --file) FILE="$2"; shift 2;;
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

mkdir -p binaries benchmarks profile

# `bench` builds `bench_evtx_dump_loop`, which the profile uses.
cargo build --release --features "fast-alloc bench" --bin evtx_dump --bin bench_evtx_dump_loop
cp target/release/evtx_dump "$BIN"

# The post stage measures both binaries.
BINS=("$PRE_BIN")
if [[ "$STAGE" == "post" ]]; then
  BINS+=("$BIN")
fi

# hyperfine discards the output of the command.
COMMANDS=()
for bin in "${BINS[@]}"; do
  COMMANDS+=("$bin ${DUMP_ARGS[*]} $FILE")
done
hyperfine -w 10 -r 20 "${COMMANDS[@]}" | tee "benchmarks/benchmark_${TAG}_${STAGE}.txt"

if [[ $CACHEGRIND -eq 1 ]]; then
  for bin in "${BINS[@]}"; do
    valgrind --tool=cachegrind --cache-sim=no --cachegrind-out-file=/dev/null \
      "$bin" "${DUMP_ARGS[@]}" "$FILE" 2>&1 >/dev/null
  done | tee "benchmarks/cachegrind_${TAG}_${STAGE}.txt"
fi

if [[ $SKIP_SAMPLY -eq 0 ]]; then
  # The loop binary parses the file many times in one process, which gives samply enough
  # samples. It always renders JSON without indentation, on the system allocator.
  PROFILE="profile/${TAG}_${STAGE}.json.gz"
  if samply record --save-only --unstable-presymbolicate -o "$PROFILE" -- \
      target/release/bench_evtx_dump_loop --file "$FILE" --loops 20 --threads 1; then
    python3 scripts/samply_extract_tables.py \
      --profile "$PROFILE" \
      --syms "${PROFILE%.gz}.syms.json" \
      --out-dir profile --label "${TAG}_${STAGE}"
  else
    echo "samply could not record a profile here (it needs perf events); the benchmark above stands." >&2
  fi
fi
