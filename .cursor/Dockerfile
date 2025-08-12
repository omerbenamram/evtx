# syntax=docker/dockerfile:1

# Dev/profiling image for building and profiling evtx_dump
# - Rust 1.88 (matches docs) on Debian bookworm (slim)
# - perf (Linux profiling), LLVM (PGO: llvm-profdata), hyperfine, perl/awk
# - cargo tools: inferno + cargo-flamegraph

FROM rust:1.88-slim-bookworm

ENV DEBIAN_FRONTEND=noninteractive \
    TZ=UTC \
    PATH=/usr/local/cargo/bin:${PATH}

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    bash \
    sudo \
    # Build toolchain
    build-essential \
    pkg-config \
    clang \
    llvm \
    # perf userland (record/script) for Linux
    linux-perf \
    # Convenience/tools used in Makefile and scripts
    hyperfine \
    perl \
    gawk \
  && rm -rf /var/lib/apt/lists/*

# Pre-install cargo tools used by Makefile targets
RUN cargo install inferno flamegraph

WORKDIR /work

# Copy manifests first to leverage Docker build cache
COPY Cargo.toml Cargo.lock ./

# Copy source and helper files
COPY src ./src
COPY Makefile ./Makefile
COPY OPTIMIZATION.md ./OPTIMIZATION.md
COPY scripts ./scripts
COPY samples ./samples

# Warm up build cache and produce a release binary with fast allocator
RUN cargo build --release --bin evtx_dump --features fast-alloc

# Default to an interactive shell with a quick hint
CMD ["bash", "-lc", "echo 'Ready. Examples: make flamegraph-prod FLAME_FILE=\"samples/security_big_sample.evtx\" FORMAT=xml DURATION=30' && exec bash"]
