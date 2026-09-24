import type { EvtxRecord, FilterOptions, TableColumn, TabularRow } from "./types";
import {
  countRecords,
  fetchTabular,
  fetchRecordByKey,
  findRowIndex,
  getSessionId,
  buildWhere,
  queryLogs,
} from "./duckdb";

export interface RowSort {
  id: string;
  desc: boolean;
}

export const PAGE_SIZE = 512;
export const MAX_CACHED_PAGES = 6;
// Sorted or filtered views re-query the whole table at most this often while a log imports.
// ponytail: fixed interval; a sort or count slower than this still delays the inserts queued
// behind it. Scale it by the last query's duration if very large logs need that.
export const IMPORT_REFRESH_MS = 2000;

const columnIds = (columns: TableColumn[]) => JSON.stringify(columns.map(({ id }) => id));

interface RecordCount {
  total: number;
  pending: Promise<void> | null;
}

export class DuckDbDataSource {
  readonly sessionId: number;
  readonly queryKey: string;
  private readonly cacheVersion = Symbol();
  private cache = {
    version: this.cacheVersion,
    pages: new Map<number, TabularRow[]>(),
    visiblePages: new Set<number>(),
  };
  private readonly pending = new Map<number, Promise<TabularRow[]>>();
  private count: RecordCount = { total: 0, pending: null };
  private readonly createdAt = performance.now();

  constructor(
    private readonly filters: FilterOptions,
    private readonly columns: TableColumn[],
    private readonly sort: RowSort | null = null,
    sessionId = getSessionId(),
    private readonly loadedRecords?: number,
    private readonly query = queryLogs,
  ) {
    this.sessionId = sessionId;
    this.queryKey = JSON.stringify([sessionId, filters, sort]);
  }

  withSort(sort: RowSort | null): DuckDbDataSource {
    if (!sort) return this;
    const sorted = new DuckDbDataSource(
      this.filters,
      this.columns,
      sort,
      this.sessionId,
      this.loadedRecords,
      this.query,
    );
    sorted.count = this.count;
    return sorted;
  }

  private sameQuery(other: DuckDbDataSource): boolean {
    return (
      this.queryKey === other.queryKey &&
      this.query === other.query &&
      columnIds(this.columns) === columnIds(other.columns)
    );
  }

  /**
   * Whether this newer snapshot should replace `shown` on screen. While importing, only an
   * unsorted, unfiltered view refreshes on every tick: its count is the import total and its
   * complete pages carry over. Other views re-sort or re-count the whole table on refresh.
   */
  replaces(shown: DuckDbDataSource, importing: boolean, now = performance.now()): boolean {
    if (!this.sameQuery(shown)) return true;
    // Same rows (e.g. a column was resized): keep the pages and count already loaded.
    if (this.loadedRecords === shown.loadedRecords) return false;
    if (!importing || now - shown.createdAt >= IMPORT_REFRESH_MS) return true;
    if (this.sort) return false;
    try {
      return !buildWhere(this.filters);
    } catch {
      return false; // Invalid filters fail every query; refreshing cannot help.
    }
  }

  /** Complete unsorted pages cannot change when the same log is appended. */
  reusePages(previous: DuckDbDataSource): void {
    if (this.sort || !this.sameQuery(previous)) return;
    this.cache = previous.cache;
    this.cache.version = this.cacheVersion;
    for (const [page, rows] of this.cache.pages) {
      if (rows.length < PAGE_SIZE) this.cache.pages.delete(page);
    }
  }

  private assertCurrent(): void {
    if (this.sessionId !== getSessionId()) {
      throw new DOMException("The log file changed.", "AbortError");
    }
  }

  async init(): Promise<void> {
    if (this.count.pending) return this.count.pending;
    const pendingCount =
      this.loadedRecords !== undefined && !buildWhere(this.filters)
        ? Promise.resolve(this.loadedRecords)
        : countRecords(this.filters, this.query);
    this.count.pending ??= pendingCount
      .then((count) => {
        this.assertCurrent();
        this.count.total = count;
      })
      .catch((cause: unknown) => {
        this.count.pending = null;
        throw cause;
      });
    return this.count.pending;
  }

  getTotalRecords(): number {
    return this.count.total;
  }

  peekRow(index: number): TabularRow | undefined {
    return this.cache.pages.get(Math.floor(index / PAGE_SIZE))?.[index % PAGE_SIZE];
  }

  setVisiblePages(first: number, last: number): void {
    this.cache.visiblePages = new Set(
      Array.from({ length: last - first + 1 }, (_, i) => first + i),
    );
  }

  async getPage(page: number): Promise<TabularRow[]> {
    this.assertCurrent();
    if (!Number.isSafeInteger(page) || page < 0) throw new Error("Invalid page");
    const cached = this.cache.pages.get(page);
    if (cached) {
      this.cache.pages.delete(page);
      this.cache.pages.set(page, cached);
      return cached;
    }
    const pending = this.pending.get(page);
    if (pending) return pending;
    const request = fetchTabular(
      this.columns,
      this.filters,
      PAGE_SIZE,
      page * PAGE_SIZE,
      this.sort,
      this.query,
    )
      .then((rows) => {
        this.assertCurrent();
        // Old full-page requests remain useful after append-only refreshes.
        // An unfinished tail must come from the current data snapshot.
        if (rows.length < PAGE_SIZE && this.cache.version !== this.cacheVersion) return rows;
        // A late offscreen request must not evict pages now on screen.
        if (this.cache.visiblePages.size && !this.cache.visiblePages.has(page)) return rows;
        this.cache.pages.set(page, rows);
        while (this.cache.pages.size > MAX_CACHED_PAGES) {
          const oldest =
            [...this.cache.pages.keys()].find((key) => !this.cache.visiblePages.has(key)) ??
            this.cache.pages.keys().next().value;
          if (oldest !== undefined) this.cache.pages.delete(oldest);
        }
        return rows;
      })
      .finally(() => this.pending.delete(page));
    this.pending.set(page, request);
    return request;
  }

  async getRow(index: number): Promise<TabularRow | undefined> {
    if (!Number.isSafeInteger(index) || index < 0) return undefined;
    const page = await this.getPage(Math.floor(index / PAGE_SIZE));
    return page[index % PAGE_SIZE];
  }

  async getRecord(key: string): Promise<EvtxRecord | null> {
    this.assertCurrent();
    const record = await fetchRecordByKey(key, this.query);
    this.assertCurrent();
    return record;
  }

  async indexOf(key: string): Promise<number | null> {
    this.assertCurrent();
    const index = await findRowIndex(key, this.filters, this.columns, this.sort, this.query);
    this.assertCurrent();
    return index;
  }
}
