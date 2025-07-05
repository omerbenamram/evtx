import type { LazyEvtxReader } from "./lazyReader";
import { insertArrowIPC, initDuckDB } from "../lib/duckdb";

export interface FullIngestOptions {
  /** AbortSignal to cancel an in-flight ingest when a new file is opened */
  signal?: AbortSignal;
  /** Size of each Arrow batch – default 10 000 */
  batchSize?: number;
}

export type IngestProgressCallback = (pct: number) => void;

/**
 * Stream the entire EVTX file via LazyEvtxReader into DuckDB using Arrow batches.
 * Progress is reported as fraction [0,1].
 */
export async function startFullIngest(
  reader: LazyEvtxReader,
  onProgress?: IngestProgressCallback,
  opts: FullIngestOptions = {}
): Promise<void> {
  const { signal } = opts;
  const { totalChunks } = await reader.getFileInfo();

  await initDuckDB();

  for (let chunkIdx = 0; chunkIdx < totalChunks; chunkIdx++) {
    if (signal?.aborted) return;
    // Retrieve Arrow IPC for the whole chunk from Rust/WASM
    const { buffer } = await reader.getArrowIPCChunk(chunkIdx);

    // Insert into DuckDB in one go – DuckDB handles chunking internally.
    if (signal?.aborted) return;
    await insertArrowIPC(buffer);

    if (onProgress) {
      const pct = (chunkIdx + 1) / totalChunks;
      const clamped = Math.min(1, pct);
      console.debug(`Full ingest progress: ${(clamped * 100).toFixed(2)}%`);
      onProgress(clamped);
    }

    // allow UI thread to breathe between chunks
    await new Promise((r) => requestAnimationFrame(() => r(null)));
  }

  if (onProgress) {
    // eslint-disable-next-line no-console
    console.debug("Full ingest progress: 100% (complete)");
    onProgress(1);
  }
}
