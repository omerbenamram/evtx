/*
 * Data source abstraction for LogTable virtual loading.
 * -----------------------------------------------------
 * A RecordDataSource hides the underlying storage/parsing mechanism and
 * exposes a simple cursor-like API that returns slices of `EvtxRecord`s.
 *
 * The initial implementation, `LazyDataSource`, wraps `LazyEvtxReader` which
 * talks to the WASM parser.  It converts a global row offset into the
 * corresponding chunk window, then delegates to `reader.getWindow()`.
 */

import type { EvtxRecord } from "./types";
import { LazyEvtxReader } from "./lazyReader";
import { logger } from "./logger";

export interface RecordDataSource {
  /** Total number of records in the log. */
  getTotal(): Promise<number>;

  /**
   * Retrieve `limit` records starting at the **global** offset. The offset is
   * zero-based across the *whole* file (not per-chunk).
   */
  getRows(offset: number, limit: number): Promise<EvtxRecord[]>;
}

/**
 * Concrete implementation backed by `LazyEvtxReader`.
 */
export class LazyDataSource implements RecordDataSource {
  private reader: LazyEvtxReader;

  /** cumulative record count at the *start* of each chunk. */
  private chunkStartOffsets: number[] = [];
  private totalRecords = 0;
  private ready: Promise<void>;

  constructor(reader: LazyEvtxReader) {
    this.reader = reader;
    this.ready = this.initialise();
  }

  private async initialise(): Promise<void> {
    const info = await this.reader.getFileInfo();
    const rawTotalBig = info.chunkRecordCounts.reduce<bigint>(
      (sum, c) => sum + BigInt(c),
      0n
    );
    const MAX_ROWS = 100_000_000; // safe upper bound for react-virtual
    let rawTotal: number;
    if (rawTotalBig > BigInt(MAX_ROWS)) {
      logger.warn("totalRecords exceeds MAX_ROWS, clamping", {
        rawTotalBig: rawTotalBig.toString(),
      });
      rawTotal = MAX_ROWS;
    } else {
      rawTotal = Number(rawTotalBig);
    }

    if (!Number.isFinite(rawTotal) || rawTotal <= 0) {
      logger.error("Invalid totalRecords computed", {
        rawTotal,
        chunkCounts: info.chunkRecordCounts,
      });
      this.totalRecords = 0;
    } else if (rawTotal > MAX_ROWS) {
      logger.warn(`totalRecords exceeds ${MAX_ROWS}, clamping`, { rawTotal });
      this.totalRecords = MAX_ROWS;
    } else {
      this.totalRecords = rawTotal;
    }

    logger.debug("Chunk record counts", {
      counts: info.chunkRecordCounts.slice(0, 20),
    });

    logger.info("LazyDataSource initialised", {
      totalRecords: this.totalRecords,
      chunks: info.chunkRecordCounts.length,
    });

    // Build prefix sum of records – chunk i starts at offset chunkStartOffsets[i]
    this.chunkStartOffsets = new Array(info.chunkRecordCounts.length);
    let running = 0;
    for (let i = 0; i < info.chunkRecordCounts.length; i++) {
      this.chunkStartOffsets[i] = running;
      running += info.chunkRecordCounts[i];
    }
  }

  async getTotal(): Promise<number> {
    await this.ready;
    return this.totalRecords;
  }

  /** Binary search helper to locate the chunk containing a global offset. */
  private findChunkIdx(offset: number): number {
    let low = 0;
    let high = this.chunkStartOffsets.length - 1;

    while (low <= high) {
      const mid = Math.floor((low + high) / 2);
      const start = this.chunkStartOffsets[mid];
      const nextStart =
        mid + 1 < this.chunkStartOffsets.length
          ? this.chunkStartOffsets[mid + 1]
          : this.totalRecords;

      if (offset >= start && offset < nextStart) {
        return mid;
      }
      if (offset < start) {
        high = mid - 1;
      } else {
        low = mid + 1;
      }
    }

    throw new RangeError(
      `Offset ${offset} out of range (total ${this.totalRecords})`
    );
  }

  async getRows(offset: number, limit: number): Promise<EvtxRecord[]> {
    logger.debug("LazyDataSource getRows", { offset, limit });
    if (limit <= 0) return [];
    await this.ready;

    const records: EvtxRecord[] = [];
    let remaining = limit;
    let currentOffset = offset;

    while (remaining > 0 && currentOffset < this.totalRecords) {
      const chunkIdx = this.findChunkIdx(currentOffset);
      const chunkStart = this.chunkStartOffsets[chunkIdx];
      const innerOffset = currentOffset - chunkStart;

      // Records left in this chunk
      const recordsInChunk =
        (chunkIdx + 1 < this.chunkStartOffsets.length
          ? this.chunkStartOffsets[chunkIdx + 1]
          : this.totalRecords) - chunkStart;

      const take = Math.min(remaining, recordsInChunk - innerOffset);

      const window = await this.reader.getWindow({
        chunkIndex: chunkIdx,
        start: innerOffset,
        limit: take,
      });

      logger.debug("Loaded window", {
        chunkIdx,
        innerOffset,
        take,
        received: window.length,
      });

      records.push(...window);
      remaining -= take;
      currentOffset += take;
    }

    return records;
  }
}
