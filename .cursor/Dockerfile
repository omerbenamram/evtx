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
    # perf userland (record/script) for Linux
    linux-perf \
    # Convenience/tools used in Makefile and scripts
    hyperfine \
    perl \
    gawk \
    # Debugging/analysis
    gdb \
    lldb \
    strace \
    # Developer ergonomics
    ripgrep \
    jq \
  && rm -rf /var/lib/apt/lists/*

# Rust developer components and cargo tools
RUN rustup component add clippy rustfmt && \
    cargo install inferno flamegraph

WORKDIR /workspace

# Do not copy source code. Cursor will clone the repository after the image is built.

# Default to an interactive shell
CMD ["bash"]
