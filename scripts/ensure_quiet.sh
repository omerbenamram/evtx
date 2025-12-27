#!/usr/bin/env bash
#
# ensure_quiet.sh - block until the system is "quiet enough" for benchmarking.
#
# Intended usage:
#   hyperfine --prepare ./scripts/ensure_quiet.sh ...
#   QUIET_CHECK=1 ./profile_comparison.sh --bench-only
#
# Thresholds (override via env):
#   QUIET_IDLE_MIN=90            # minimum CPU idle percentage
#   QUIET_LOAD1_MAX=2.0          # maximum 1-minute load average
#   QUIET_STABLE_SAMPLES=3       # consecutive passing samples required
#   QUIET_SAMPLE_INTERVAL_SEC=0.25
#   QUIET_MAX_WAIT_SEC=60
#   QUIET_VERBOSE=0              # set to 1 to print every sample
#
# Notes:
# - `hyperfine --prepare` does NOT include prepare time in the measured timings, so
#   waiting for quiet won’t skew results, it just prevents noisy runs.

set -euo pipefail

IDLE_MIN="${QUIET_IDLE_MIN:-90}"
LOAD1_MAX="${QUIET_LOAD1_MAX:-2.0}"
STABLE_SAMPLES="${QUIET_STABLE_SAMPLES:-3}"
SAMPLE_INTERVAL_SEC="${QUIET_SAMPLE_INTERVAL_SEC:-0.25}"
MAX_WAIT_SEC="${QUIET_MAX_WAIT_SEC:-60}"
VERBOSE="${QUIET_VERBOSE:-0}"

script_name="$(basename "$0")"

die() {
    echo "[$script_name] error: $*" >&2
    exit 2
}

get_cpu_idle_percent() {
    local os
    os="$(uname -s)"

    if [[ "$os" == "Darwin" ]]; then
        # Use the *second* sample from `top -l 2` to avoid the "since boot/last call" artifact.
        local line
        line="$(top -l 2 -n 0 | grep '^CPU usage' | tail -n 1 || true)"
        if [[ -z "$line" ]]; then
            die "failed to read CPU usage from top (Darwin)"
        fi
        # Example: "CPU usage: 12.89% user, 15.2% sys, 72.8% idle"
        echo "$line" | sed -E 's/.* ([0-9.]+)% idle.*/\1/'
        return 0
    fi

    if [[ "$os" == "Linux" ]]; then
        # Compute idle% from /proc/stat deltas.
        # Fields: user nice system idle iowait irq softirq steal guest guest_nice
        local u1 n1 s1 i1 w1 irq1 sirq1 st1
        local u2 n2 s2 i2 w2 irq2 sirq2 st2
        read -r _ u1 n1 s1 i1 w1 irq1 sirq1 st1 _ < /proc/stat || die "failed to read /proc/stat"
        sleep 0.10
        read -r _ u2 n2 s2 i2 w2 irq2 sirq2 st2 _ < /proc/stat || die "failed to read /proc/stat (2)"

        local idle1=$((i1 + w1))
        local idle2=$((i2 + w2))
        local total1=$((u1 + n1 + s1 + i1 + w1 + irq1 + sirq1 + st1))
        local total2=$((u2 + n2 + s2 + i2 + w2 + irq2 + sirq2 + st2))
        local didle=$((idle2 - idle1))
        local dtotal=$((total2 - total1))

        if (( dtotal <= 0 )); then
            die "invalid /proc/stat delta"
        fi

        awk -v idle="$didle" -v total="$dtotal" 'BEGIN { printf "%.2f", (idle / total) * 100.0 }'
        return 0
    fi

    die "unsupported OS for idle sampling: $os"
}

get_load1() {
    local os
    os="$(uname -s)"

    if [[ "$os" == "Darwin" ]]; then
        # Example: "{ 5.34 5.49 4.95 }"
        sysctl -n vm.loadavg | sed -E 's/^\{ ([0-9.]+) .*/\1/'
        return 0
    fi

    if [[ "$os" == "Linux" ]]; then
        awk '{print $1}' /proc/loadavg
        return 0
    fi

    die "unsupported OS for load sampling: $os"
}

float_ge() {
    # $1 >= $2
    awk -v a="$1" -v b="$2" 'BEGIN { exit !(a >= b) }'
}

float_le() {
    # $1 <= $2
    awk -v a="$1" -v b="$2" 'BEGIN { exit !(a <= b) }'
}

deadline=$((SECONDS + MAX_WAIT_SEC))
ok_streak=0
samples=0

while true; do
    samples=$((samples + 1))

    idle="$(get_cpu_idle_percent)"
    load1="$(get_load1)"

    if float_ge "$idle" "$IDLE_MIN" && float_le "$load1" "$LOAD1_MAX"; then
        ok_streak=$((ok_streak + 1))
    else
        ok_streak=0
    fi

    if [[ "$VERBOSE" != "0" ]]; then
        echo "[$script_name] idle=${idle}% (min ${IDLE_MIN}%) load1=${load1} (max ${LOAD1_MAX}) streak=${ok_streak}/${STABLE_SAMPLES}" >&2
    fi

    if (( ok_streak >= STABLE_SAMPLES )); then
        exit 0
    fi

    if (( SECONDS >= deadline )); then
        echo "[$script_name] system not quiet enough after ${MAX_WAIT_SEC}s." >&2
        echo "[$script_name] last sample: idle=${idle}% (min ${IDLE_MIN}%), load1=${load1} (max ${LOAD1_MAX})." >&2
        echo "[$script_name] top CPU processes:" >&2
        ps -Ao %cpu,pid,command | sed 1d | sort -nr | head -n 10 >&2 || true
        exit 1
    fi

    sleep "$SAMPLE_INTERVAL_SEC"
done

