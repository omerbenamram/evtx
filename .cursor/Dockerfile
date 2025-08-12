# syntax=docker/dockerfile:1

# Cursor dev image: installs developer tools only, without copying repo code
# - Rust 1.88 on Debian bookworm (slim)
# - Tools: build-essential, pkg-config, clang/llvm (inc. llvm-profdata), perf, hyperfine
# - Debugging: gdb, lldb, strace; Dev UX: ripgrep, jq, curl, git, bash
# - Cargo tools: clippy, rustfmt, inferno, cargo-flamegraph

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
    # Convenience/tools used in Makefile and scripts
    perl \
    gawk \
    # Debugging/analysis (gdb/strace)
    gdb \
    strace \
    # Developer ergonomics
    ripgrep \
    jq \
  && rm -rf /var/lib/apt/lists/*

# Try to install perf if available in this Debian release. Do not fail if missing.
RUN apt-get update && ( \
      apt-get install -y --no-install-recommends linux-perf \
   || apt-get install -y --no-install-recommends linux-perf-6.1 \
   || echo "linux-perf not available; perf-based profiling will require host install" ) \
   && rm -rf /var/lib/apt/lists/*

# Try to install lldb best-effort; skip if not present in this suite
RUN apt-get update && ( \
      apt-get install -y --no-install-recommends lldb \
   || apt-get install -y --no-install-recommends lldb-16 \
   || apt-get install -y --no-install-recommends lldb-15 \
   || apt-get install -y --no-install-recommends lldb-14 \
   || echo "lldb not available in this base image" ) \
   && rm -rf /var/lib/apt/lists/*

# Rust developer components and cargo tools
RUN rustup component add clippy rustfmt && \
    cargo install inferno flamegraph hyperfine

WORKDIR /workspace

# Do not copy source code. Cursor will clone the repository after the image is built.

# Default to an interactive shell
CMD ["bash"]
