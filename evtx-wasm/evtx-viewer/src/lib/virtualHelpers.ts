import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer, Virtualizer } from "@tanstack/react-virtual";
import type { EvtxRecord } from "./types";
import { DuckDbDataSource } from "./duckDbDataSource";
import { logger } from "./logger";

export interface ChunkVirtualizer {
  /** Attach this to the *scrollable* element that should drive scrolling */
  containerRef: React.MutableRefObject<HTMLDivElement | null>;
  /** Underlying tanstack virtualizer (per-chunk).  Consumers usually only
   *  need `getVirtualItems()` and `getTotalSize()`. */
  virtualizer: Virtualizer<HTMLDivElement, Element>;
  /** Map of *loaded* chunks ⇒ their record arrays */
  chunkRows: Map<number, EvtxRecord[]>;
  /** prefix[i] === global row offset at *start* of chunk i */
  prefix: number[];
  /** Upper-bound on total number of rows (becomes exact once all chunks
   *  were loaded).  Safe to use for virtualizer row counts etc. */
  totalRows: number;
  /** Explicitly trigger loading of a chunk (hook does this automatically
   *  for visible chunks). */
  ensureChunk: (idx: number) => void;
}

interface UseChunkVirtualizerOpts {
  dataSource: DuckDbDataSource;
  /** Fixed pixel height of *one* rendered row. */
  rowHeight: number;
  /** Estimated #records per chunk *before* we actually load it. */
  estimateRowsPerChunk?: number;
  /** Overscan #chunks for the tanstack virtualizer. */
  overscanChunks?: number;
  /** Optional predicate – only rows that return true will be kept. */
  filterFn?: (rec: EvtxRecord) => boolean;
}

export function useChunkVirtualizer({
  dataSource,
  rowHeight,
  estimateRowsPerChunk = 4000,
  overscanChunks = 2,
  filterFn,
}: UseChunkVirtualizerOpts): ChunkVirtualizer {
  // --- bookkeeping --------------------------------------------------------
  const [chunkCount, setChunkCount] = useState(0);
  const [chunkRows, setChunkRows] = useState<Map<number, EvtxRecord[]>>(
    () => new Map()
  );
  const loadingChunks = useRef<Set<number>>(new Set());

  // Exact per-chunk record counts discovered during init (may be empty until
  // dataSource.init() resolves).
  const [chunkRecordCounts, setChunkRecordCounts] = useState<number[]>([]);

  // No dynamic global estimate; we rely on header counts until real chunk is loaded.

  // Initialise chunk count *once*
  useEffect(() => {
    let mounted = true;
    (async () => {
      await dataSource.init();
      if (mounted) {
        const cnt = dataSource.getChunkCount();
        const counts = dataSource.getChunkRecordCounts();
        logger.info("ChunkDataSource initialised", {
          chunks: cnt,
          records: counts.reduce((a, b) => a + b, 0),
        });
        setChunkCount(cnt);
        setChunkRecordCounts(counts);
      }
    })();
    return () => {
      mounted = false;
    };
  }, [dataSource]);

  // --- virtualiser --------------------------------------------------------
  const containerRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: chunkCount,
    getScrollElement: () => containerRef.current,
    estimateSize: (idx) => {
      const header = chunkRecordCounts[idx];
      return rowHeight * (header ?? estimateRowsPerChunk);
    },
    overscan: overscanChunks,
  });

  // Reset all cached measurements & state when the *dataSource* instance changes
  useEffect(() => {
    // Clear any in-flight loads for the previous data source
    loadingChunks.current.clear();

    // Drop any rows we already downloaded so the new file starts fresh
    setChunkRows(new Map());
    setChunkCount(0);

    // Tell the virtualizer to forget all previously sized items
    virtualizer.measure();

    // Forget any cached record counts until the new file's init() runs
    setChunkRecordCounts([]);

    // Reset scroll offset to the top so that the first chunk becomes visible
    if (containerRef.current) {
      containerRef.current.scrollTop = 0;
    }
    virtualizer.scrollToOffset(0);

    logger.info("dataSourceChanged – cache cleared & virtualizer reset");
  }, [dataSource, virtualizer, estimateRowsPerChunk]);

  // When the filter predicate changes we need to re-evaluate *all* already
  // loaded chunks so that their heights & row counts reflect the new filter.
  useEffect(() => {
    if (!filterFn) {
      // If no filter, nothing to update – but we still clear so that each
      // chunk will be re-sized based on its full record count.
      setChunkRows(new Map());
      virtualizer.measure();
      return;
    }

    // Re-filter synchronously if we already have the original rows cached in
    // the data source (which we should).  Otherwise chunks will update lazily
    // the next time they load.
    (async () => {
      const newMap = new Map<number, EvtxRecord[]>();
      for (const idx of chunkRows.keys()) {
        const original = await dataSource.getChunk(idx);
        newMap.set(idx, original.filter(filterFn));

        // Update the virtualizer item size immediately.
        virtualizer.resizeItem(idx, newMap.get(idx)!.length * rowHeight);
      }
      setChunkRows(newMap);
    })();
  }, [filterFn, dataSource, rowHeight, virtualizer]);

  // --- chunk loader helper -----------------------------------------------
  const ensureChunk = useCallback(
    (idx: number) => {
      if (chunkRows.has(idx) || loadingChunks.current.has(idx)) return;
      logger.debug("ensureChunk", { idx });
      loadingChunks.current.add(idx);
      void dataSource.getChunk(idx).then((records) => {
        const filtered = filterFn ? records.filter(filterFn) : records;

        setChunkRows((prev) => new Map(prev).set(idx, filtered));

        logger.debug("chunkLoaded", {
          idx,
          original: records.length,
          kept: filtered.length,
        });
        logger.info("chunkLoaded", {
          idx,
          original: records.length,
          kept: filtered.length,
        });
        loadingChunks.current.delete(idx);

        const estimatedPx =
          (chunkRecordCounts[idx] ?? estimateRowsPerChunk) * rowHeight;
        const exact = filtered.length * rowHeight;

        logger.debug("chunkResize", {
          idx,
          estimatedPx,
          exactPx: exact,
          diffPx: exact - estimatedPx,
        });

        virtualizer.resizeItem(idx, exact);

        logger.debug("virtualizerTotalSize", {
          idx,
          totalSizePx: virtualizer.getTotalSize(),
        });

        /*
         * We previously called `virtualizer.measure()` when the **last** chunk
         * finished loading in an attempt to force a total-size recalculation.
         *
         * Unfortunately that had the opposite effect: `measure()` blows away
         * all `resizeItem()` overrides we just applied and falls back to the
         * (often wildly over-estimated) `estimateSize` callback, which in turn
         * made the virtual content much taller than the real data -> a large
         * blank area at the bottom of the table.
         *
         * Because every chunk already receives an exact `resizeItem()` as soon
         * as it loads, the total size is *already* correct at this point, so
         * running `measure()` is unnecessary – and actively harmful.
         */
      });
    },
    [
      chunkRows,
      dataSource,
      rowHeight,
      virtualizer,
      chunkCount,
      chunkRecordCounts,
      estimateRowsPerChunk,
      filterFn,
    ]
  );

  // --- prefix / total rows ------------------------------------------------
  const prefix = useMemo(() => {
    const arr: number[] = new Array(chunkCount);
    let offset = 0;
    for (let i = 0; i < chunkCount; i++) {
      arr[i] = offset;
      const rowsInChunk =
        chunkRows.get(i)?.length ??
        chunkRecordCounts[i] ??
        estimateRowsPerChunk;
      offset += rowsInChunk;
    }
    logger.debug("prefixRecalc", { chunkCount, lastOffset: offset });
    return arr;
  }, [chunkCount, chunkRows, chunkRecordCounts]);

  const totalRows = useMemo(() => {
    if (chunkCount === 0) return 0;
    const last = chunkCount - 1;
    const total =
      prefix[last] +
      (chunkRows.get(last)?.length ??
        Math.min(
          estimateRowsPerChunk,
          chunkRecordCounts[last] ?? estimateRowsPerChunk
        ));
    logger.debug("totalRows", { total });
    return total;
  }, [chunkCount, prefix, chunkRows, chunkRecordCounts]);

  // Log whenever totalRows changes so we can correlate with blank space
  useEffect(() => {
    logger.debug("totalRowsChanged", {
      totalRows,
      virtualizerTotalPx: virtualizer.getTotalSize(),
    });
  }, [totalRows, virtualizer]);

  // Whenever virtual items change, proactively load the *visible* chunks
  useEffect(() => {
    const v = virtualizer.getVirtualItems();
    if (v.length) {
      logger.debug("virtualItems", {
        first: v[0].index,
        last: v[v.length - 1].index,
        count: v.length,
      });
    }
    v.forEach((vi) => ensureChunk(vi.index));
  }, [virtualizer.getVirtualItems(), ensureChunk]);

  // -----------------------------------------------------------------------
  return {
    containerRef,
    virtualizer,
    chunkRows,
    prefix,
    totalRows,
    ensureChunk,
  };
}

export interface SliceConfig {
  viewportStart: number;
  viewportHeight: number;
  chunkTop: number;
  chunkHeight: number;
  rowHeight: number;
  bufferRows: number;
  maxRows: number;
  recordCount: number;
}

/**
 * Compute the [startRow, endRow] (inclusive) within a chunk that intersect the
 * viewport plus buffer. Returns `null` if the chunk is entirely outside the
 * buffered viewport.
 */
export function computeSliceRows(cfg: SliceConfig): [number, number] | null {
  const {
    viewportStart,
    viewportHeight,
    chunkTop,
    chunkHeight,
    rowHeight,
    bufferRows,
    maxRows,
    recordCount,
  } = cfg;

  const bufferPx = bufferRows * rowHeight;
  const viewportEnd = viewportStart + viewportHeight;

  const chunkBottom = chunkTop + chunkHeight;

  // Entire chunk outside buffered viewport
  if (
    viewportEnd + bufferPx <= chunkTop ||
    viewportStart - bufferPx >= chunkBottom
  ) {
    return null;
  }

  // Intersection bounds in pixels within chunk
  const intersectTopPx =
    Math.max(viewportStart - bufferPx, chunkTop) - chunkTop;
  const intersectBottomPx =
    Math.min(viewportEnd + bufferPx, chunkBottom) - chunkTop;

  // No intersection if bottom is above top
  if (intersectBottomPx <= 0 || intersectTopPx >= chunkHeight) {
    return null;
  }

  let startRow = Math.floor(intersectTopPx / rowHeight);
  let endRow = Math.ceil(intersectBottomPx / rowHeight) - 1; // inclusive

  // Clamp to valid record indices
  startRow = Math.min(Math.max(0, startRow), recordCount - 1);
  endRow = Math.min(recordCount - 1, Math.max(startRow, endRow));

  // Enforce max rows window
  if (endRow - startRow + 1 > maxRows) {
    endRow = startRow + maxRows - 1;
  }

  // If after clamping we ended with an empty range, skip rendering
  if (startRow > endRow) {
    return null;
  }

  return [startRow, endRow];
}
