import { insertArrowIPC } from "./duckdb";
import type { EvtxWorkerReader } from "./workerReader";
import { EvtxChunkError } from "./workerProtocol";

/** One batch in flight: parsing cannot outrun the database or retain old batches. */
export async function startFullIngest(
  reader: Pick<EvtxWorkerReader, "getArrowIPCChunk">,
  totalChunks: number,
  signal: AbortSignal,
  onProgress: (progress: number, records: number) => void,
  onWarning: (warning: string) => void,
  writeBatch = insertArrowIPC,
): Promise<number> {
  let records = 0;
  for (let index = 0; index < totalChunks; index++) {
    signal.throwIfAborted();
    let chunk;
    try {
      chunk = await reader.getArrowIPCChunk(index);
    } catch (error) {
      signal.throwIfAborted();
      if (!(error instanceof EvtxChunkError)) throw error;
      onWarning(`Chunk ${index + 1}: ${error.message}`);
    }
    signal.throwIfAborted();
    if (chunk) {
      for (const warning of chunk.errors) onWarning(`Chunk ${index + 1}: ${warning}`);
      if (chunk.rows > 0) {
        await writeBatch(new Uint8Array(chunk.buffer));
        records += chunk.rows;
      }
    }
    signal.throwIfAborted();
    onProgress((index + 1) / totalChunks, records);
  }
  onProgress(1, records);
  return records;
}
