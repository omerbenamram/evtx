FLAME_FILE ?= samples/security_big_sample.evtx
FORMAT ?= json
DURATION ?= 30
BIN ?= ./target/release/evtx_dump

.PHONY: flamegraph-prod
flamegraph-prod:
	@echo "Building release binary with fast allocator..."
	cargo build --release --features fast-alloc
	@echo "Cleaning up previous trace files..."
	@rm -rf cargo-flamegraph.trace
	BIN="$(BIN)" FLAME_FILE="$(FLAME_FILE)" FORMAT="$(FORMAT)" DURATION="$(DURATION)" \
		bash scripts/flamegraph_prod.sh


