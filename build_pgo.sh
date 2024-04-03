echo "Building binary for instrumented run"

# define toolchain variable
TOOLCHAIN="stable-x86_64-unknown-linux-gnu"
TARGET="x86_64-unknown-linux-gnu"

echo "Cleaning up old build artifacts"
cargo clean
rm -rf /tmp/pgo-data

PATH=$HOME/.rustup/toolchains/$TOOLCHAIN/lib/rustlib/$TARGET/bin:$PATH
RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" \
    cargo build --release --target $TARGET --features fast-alloc

echo "Running instrumented binary"
for i in $(find samples -name "*.evtx"); do
    echo "Processing $i"
    ./target/$TARGET/release/evtx_dump -t 1 -o json $i 1>/dev/null 2>&1
    ./target/$TARGET/release/evtx_dump -t 1 -o xml $i 1>/dev/null 2>&1
    ./target/$TARGET/release/evtx_dump -t 8 -o json $i 1>/dev/null 2>&1
done

echo "Merging profile data"
llvm-profdata merge -o /tmp/pgo-data/merged.profdata /tmp/pgo-data

echo "Building binary with profile data"
RUSTFLAGS="-Cprofile-use=/tmp/pgo-data/merged.profdata" \
    cargo build --release --target $TARGET --features fast-alloc
