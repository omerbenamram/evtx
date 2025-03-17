# CLAUDE.md - Rust EVTX Parser

## Build Commands
- Build: `cargo build --release`
- Build with fast allocator: `cargo build --release --features fast-alloc`
- Build with multithreading: `cargo build --release --features multithreading`
- PGO build (Linux only): `./build_pgo.sh`

## Test Commands
- Run all tests: `cargo test`
- Run specific test: `cargo test test_name`
- Run tests with logging: `RUST_LOG=debug cargo test`
- Benchmarks: `cargo bench`

## Run Commands
- Process EVTX file: `evtx_dump [options] <evtx_file>`
- Output as JSON: `evtx_dump -o json <evtx_file>`
- Write to file: `evtx_dump -f <output_file> -o json <input_file>`

## Code Style
- Uses standard Rust conventions (snake_case functions, CamelCase types)
- Forbids unsafe code with `#![forbid(unsafe_code)]`
- Error handling with thiserror and contextual information
- Module structure: lib.rs exports public API, internal modules below
- Imports: std first, then external crates, then internal modules
- Documentation: Doc comments on public API with examples
- Tests: Uses insta for snapshot testing
