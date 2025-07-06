/* eslint-disable @typescript-eslint/no-explicit-any */
// DuckDbDataSource.ts – virtual-table backend that pages rows directly from
// DuckDB based on current filters.

import type { FilterOptions, ColumnSpec } from "./types";
import { countRecords, fetchTabular } from "./duckdb";

/**
 * Page-oriented data source for use with `useChunkVirtualizer`.  It splits the
 * full result set into fixed-size "chunks" (pages).  Each chunk is fetched on
 * demand from DuckDB using LIMIT/OFFSET so filtering is done entirely in SQL.
 */
export class DuckDbDataSource {
  private readonly filters: FilterOptions;
  private readonly columns: ColumnSpec[];
  private readonly PAGE = 4000; // match old chunk size
  private totalRecords: number | null = null;

  constructor(filters: FilterOptions, columns: ColumnSpec[]) {
    this.filters = filters;
    this.columns = columns;
  }

  /** Prepare row count so virtualiser knows total height */
  async init(): Promise<void> {
    if (this.totalRecords !== null) return; // already ready
    this.totalRecords = await countRecords(this.filters);
  }

  /** Number of logical chunks */
  getChunkCount(): number {
    if (this.totalRecords === null) return 0;
    return Math.ceil(this.totalRecords / this.PAGE);
  }

  /** Per-chunk header counts used by virtualiser’s initial estimate */
  getChunkRecordCounts(): number[] {
    const cnt = this.getChunkCount();
    if (cnt === 0 || this.totalRecords === null) return [];
    const arr = new Array(cnt).fill(this.PAGE);
    // Adjust last chunk to remaining rows
    const fullBeforeLast = (cnt - 1) * this.PAGE;
    arr[cnt - 1] = this.totalRecords - fullBeforeLast;
    return arr;
  }

  /** Exact record count for a specific chunk (unknown until init) */
  // Provided for structural compatibility with ChunkDataSource – returns
  // PAGE for all but the last chunk (after init).  Callers usually only need
  // getChunkRecordCounts() but some may call this for convenience.
  getChunkRecordCount(idx: number): number | undefined {
    const counts = this.getChunkRecordCounts();
    return counts[idx];
  }

  /** Absolute total #records across all chunks */
  getTotalRecords(): number {
    return this.totalRecords ?? 0;
  }

  /** Fetch one page */
  async getChunk(idx: number): Promise<any[]> {
    // Changed EvtxRecord[] to any[] as EvtxRecord is no longer imported
    const offset = idx * this.PAGE;
    return fetchTabular(this.columns, this.filters, this.PAGE, offset);
  }
}
