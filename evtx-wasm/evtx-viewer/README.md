# EVTX Viewer

A Windows-style event log explorer. Files are parsed locally by Rust/WASM in a dedicated worker, inserted into DuckDB as Arrow batches, and displayed in a virtualized table.

## Run

From the repository root:

```sh
./run_viewer.sh
```

This builds the WASM bindings, installs the frontend dependencies from `bun.lock`, and starts Vite at `http://localhost:3000`. Requires Rust, `wasm-pack`, and Bun.

## Investigate a log

- Open or drop an `.evtx` file, or use **Try example log**.
- Events appear while import continues. **Cancel import** keeps already imported events; **Restart import** starts again.
- Search plain text or fields such as `event_id:4624`, `provider:"Microsoft-Windows-Security-Auditing"`, and `computer:HOST`.
- Use the timeline to select a UTC interval, or enter precise start/end times.
- Sort a column, filter values, and add event fields as columns from the details pane.
- Save searches and column layouts locally. Recent logs are stored in IndexedDB.
- Export matching events as JSON; **Save Original Log As** downloads the original EVTX.

Searches and filters apply to events imported so far. Import errors and skipped records are shown explicitly. Large inputs still need enough browser/WASM memory to hold the input file and the local database; background parsing does not remove that limit.

## Check

```sh
cd evtx-wasm/evtx-viewer
bun run check # Oxfmt, Oxlint + Anti Slop, TypeScript and tests
bun run build
```

Parser regression (from the repository root):

```sh
cargo test --manifest-path evtx-wasm/Cargo.toml --lib
```

## Data flow

`File → parser worker → one Arrow batch → DuckDB worker → bounded page cache → visible rows`

A new file cancels its predecessor and waits for any submitted database insert before clearing the previous session. Raw event JSON is fetched when selected.
