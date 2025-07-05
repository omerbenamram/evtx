import { LazyEvtxReader } from "./lazyReader";
import type { EvtxRecord } from "./types";
import { logger } from "./logger";

export class ChunkDataSource {
  private reader: LazyEvtxReader;
  private chunkCount = 0;
  private chunkRecordCounts: number[] = [];
  private totalRecords = 0;
  private cache: Map<number, EvtxRecord[]> = new Map();

  constructor(reader: LazyEvtxReader) {
    this.reader = reader;
  }

  /** Initialise by querying file info to discover number of chunks. */
  async init() {
    const info = await this.reader.getFileInfo();
    this.chunkCount = info.totalChunks;
    this.chunkRecordCounts = info.chunkRecordCounts;
    this.totalRecords = info.chunkRecordCounts.reduce((a, b) => a + b, 0);
    logger.info("ChunkDataSource ready", { chunks: this.chunkCount });
  }

  getChunkCount(): number {
    return this.chunkCount;
  }

  /** Exact record count for a specific chunk (if known) */
  getChunkRecordCount(idx: number): number | undefined {
    return this.chunkRecordCounts[idx];
  }

  /** Full array of record counts (length == chunkCount) */
  getChunkRecordCounts(): number[] {
    return this.chunkRecordCounts;
  }

  /** Absolute #records across the whole file */
  getTotalRecords(): number {
    return this.totalRecords;
  }

  async getChunk(idx: number): Promise<EvtxRecord[]> {
    const cached = this.cache.get(idx);
    if (cached) return cached;

    const records = await this.reader.getWindow({
      chunkIndex: idx,
      start: 0,
      limit: 0,
    });
    this.cache.set(idx, records);
    logger.debug("Chunk loaded", { idx, records: records.length });
    return records;
  }
}
