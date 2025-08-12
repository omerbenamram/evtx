SHELL := /bin/bash

# Configuration
OS := $(shell uname -s)
OUT_DIR ?= profile
INPUT ?= ./samples/security_big_sample.evtx
# Arguments passed to the profiled binary
RUN_ARGS ?= -t 1 -o jsonl $(INPUT)
# Cargo features (default to fast allocator for benchmarking/profiling)
FEATURES ?= fast-alloc
# Sampling duration (seconds) for macOS 'sample'
DURATION ?= 30
# Sampling frequency for Linux 'perf'
FREQ ?= 997
# Explicit prod variables
FORMAT ?= jsonl
FLAME_FILE ?= $(INPUT)

# Paths
BINARY := ./target/release/evtx_dump
NO_INDENT_ARGS := --no-indent --dont-show-record-number

# FlameGraph scripts (more robust for macOS `sample` output)
FLAMEGRAPH_REPO_URL ?= https://github.com/brendangregg/FlameGraph.git
FLAMEGRAPH_DIR ?= scripts/FlameGraph

.PHONY: all deps build run folded flamegraph folded-prod flamegraph-prod clean install-flamegraph bench-refs

all: flamegraph-prod

deps:
	# Tools used: inferno (collapse + flamegraph) and cargo flamegraph (optional)
	@which inferno-flamegraph >/dev/null 2>&1 || cargo install inferno
	@which cargo-flamegraph >/dev/null 2>&1 || cargo install flamegraph

build:
	cargo build --release --features $(FEATURES)

run: build
	$(BINARY) $(RUN_ARGS)

# Produce collapsed stacks at $(OUT_DIR)/stacks.folded
folded: build
	@mkdir -p $(OUT_DIR)
ifeq ($(OS),Darwin)
	# macOS: sample the running process and collapse
	( $(BINARY) $(RUN_ARGS) >/dev/null & echo $$! > $(OUT_DIR)/pid )
	sleep 1
	# Capture to file via tee; allow process to exit during sampling without failing the make
	sample $$(cat $(OUT_DIR)/pid) $(DURATION) -mayDie | tee $(OUT_DIR)/sample.txt >/dev/null 2>&1 || true
	-kill $$(cat $(OUT_DIR)/pid) >/dev/null 2>&1 || true
	inferno-collapse-sample < $(OUT_DIR)/sample.txt > $(OUT_DIR)/stacks.folded
else
	# Linux: record with perf and collapse
	sudo perf record -F $(FREQ) -g -- $(BINARY) $(RUN_ARGS) >/dev/null
	perf script > $(OUT_DIR)/perf.script
	inferno-collapse-perf < $(OUT_DIR)/perf.script > $(OUT_DIR)/stacks.folded
endif
	@echo "Collapsed stacks written to $(OUT_DIR)/stacks.folded"

# Generate a flamegraph SVG from collapsed stacks
flamegraph: folded
	@mkdir -p $(OUT_DIR)
	inferno-flamegraph < $(OUT_DIR)/stacks.folded > $(OUT_DIR)/flamegraph.svg
	@echo "Flamegraph written to $(OUT_DIR)/flamegraph.svg"



clean:
	rm -rf $(OUT_DIR)

# --- PROD targets ---
# Clean profile dir, run with -t 1 and selected FORMAT/FLAME_FILE, and output flamegraph
install-flamegraph:
	@mkdir -p scripts
	@if [ ! -d "$(FLAMEGRAPH_DIR)" ]; then \
	  echo "Cloning FlameGraph scripts..."; \
	  git clone "$(FLAMEGRAPH_REPO_URL)" "$(FLAMEGRAPH_DIR)" >/dev/null; \
	else \
	  echo "FlameGraph already present"; \
	fi

folded-prod: build install-flamegraph
	@rm -rf $(OUT_DIR)
	@mkdir -p $(OUT_DIR)
ifeq ($(OS),Darwin)
  ( $(BINARY) -t 1 -o $(FORMAT) $(NO_INDENT_ARGS) $(FLAME_FILE) >/dev/null 2>&1 & echo $$! > $(OUT_DIR)/pid )
		# Start sampling immediately; -mayDie tolerates process exit during sampling
		sample $$(cat $(OUT_DIR)/pid) $(DURATION) -mayDie | tee $(OUT_DIR)/sample.txt >/dev/null 2>&1 || true
		@if kill -0 $$(cat $(OUT_DIR)/pid) >/dev/null 2>&1; then kill -INT $$(cat $(OUT_DIR)/pid) >/dev/null 2>&1 || true; fi
		@wait $$(cat $(OUT_DIR)/pid) 2>/dev/null || true
		awk -f "$(FLAMEGRAPH_DIR)/stackcollapse-sample.awk" "$(OUT_DIR)/sample.txt" > "$(OUT_DIR)/stacks.folded"
else
  sudo perf record -F $(FREQ) -g -- $(BINARY) -t 1 -o $(FORMAT) $(NO_INDENT_ARGS) $(FLAME_FILE) >/dev/null
	perf script > $(OUT_DIR)/perf.script
	inferno-collapse-perf < $(OUT_DIR)/perf.script > $(OUT_DIR)/stacks.folded
endif
	@echo "Collapsed stacks written to $(OUT_DIR)/stacks.folded"

flamegraph-prod: folded-prod
	@mkdir -p $(OUT_DIR)
	# Prefer Brendan Gregg's generator for compatibility with stackcollapse output
	"$(FLAMEGRAPH_DIR)/flamegraph.pl" "$(OUT_DIR)/stacks.folded" > "$(OUT_DIR)/flamegraph.svg"
	@echo "Flamegraph written to $(OUT_DIR)/flamegraph.svg"
	@echo "Computing hotspot summaries (top_leaf, top_titles)..."
	@awk '{ \
	  if (match($$0, / ([0-9]+)$$/)) { count=substr($$0, RSTART+1, RLENGTH-1) } else { count=1 } \
	  line=$$0; sub(/ [0-9]+$$/, "", line); \
	  n=split(line, frames, ";"); \
	  leaf=frames[n]; \
	  leaf_counts[leaf]+=count; \
	} END { \
	  for (l in leaf_counts) printf "%12d %s\n", leaf_counts[l], l; \
	}' $(OUT_DIR)/stacks.folded | sort -nr > $(OUT_DIR)/top_leaf.txt
	# Parse flamegraph SVG titles and capture percent even when sample count is present
	@perl -ne 'if (/<title>([^<]+) \((?:\d+(?:\.\d+)?\s+samples,\s+)?(\d+(?:\.\d+)?)%\)/) { print $$2, " ", $$1, "\n" }' "$(OUT_DIR)/flamegraph.svg" | sort -nr | head -n 30 > "$(OUT_DIR)/top_titles.txt"
	@echo "Top summaries written to $(OUT_DIR)/top_leaf.txt and $(OUT_DIR)/top_titles.txt"

# --- Reproducible benchmarking between two git refs (no stashing) ---
bench-refs:
	@bash -eu -o pipefail -c '\
	  REPO="$$PWD"; \
	  CLEAN_REF="$${CLEAN_REF:?set CLEAN_REF=<git-ref-for-clean>}"; \
	  MOD_REF="$${MOD_REF:?set MOD_REF=<git-ref-for-mod>}"; \
	  TS=$$(date -u +%Y%m%dT%H%M%SZ); \
	  mkdir -p "$$REPO/binaries" "$$REPO/benchmarks" "$$REPO/tmp/worktrees"; \
	  # Clean worktree build
	  CWT="$$REPO/tmp/worktrees/clean-$${CLEAN_REF//\//-}-$${TS}"; \
	  git worktree add --force --detach "$$CWT" "$$CLEAN_REF" >/dev/null; \
	  ( cd "$$CWT" && cargo build --release >/dev/null ); \
	  CLEAN_HASH=$$(git -C "$$CWT" rev-parse --short HEAD); \
	  CLEAN_BIN="$$REPO/binaries/evtx_dump_$${CLEAN_HASH}_$${TS}_clean"; \
	  cp "$$CWT/target/release/evtx_dump" "$$CLEAN_BIN"; \
	  # Mod worktree build
	  MWT="$$REPO/tmp/worktrees/mod-$${MOD_REF//\//-}-$${TS}"; \
	  git worktree add --force --detach "$$MWT" "$$MOD_REF" >/dev/null; \
	  ( cd "$$MWT" && cargo build --release >/dev/null ); \
	  MOD_HASH=$$(git -C "$$MWT" rev-parse --short HEAD); \
	  MOD_BIN="$$REPO/binaries/evtx_dump_$${MOD_HASH}_$${TS}_mod"; \
	  cp "$$MWT/target/release/evtx_dump" "$$MOD_BIN"; \
	  # Benchmark pair
	  "$${REPO}/scripts/run_benchmark_pair.sh" "$$CLEAN_BIN" "$$MOD_BIN" "$$REPO/samples/security_big_sample.evtx"; \
	  # Cleanup worktrees
	  git worktree remove --force "$$CWT" >/dev/null; \
	  git worktree remove --force "$$MWT" >/dev/null; \
	  git worktree prune >/dev/null; \
	  echo "Clean: $$CLEAN_BIN"; echo "Mod  : $$MOD_BIN"; \
	'

