import { useCallback, useEffect, useRef, useState } from "react";
import { LazyEvtxReader } from "../lib/lazyReader";
import { EvtxParser } from "../lib/parser";
import { DuckDbDataSource } from "../lib/duckDbDataSource";
import type {
  BucketCounts,
  EvtxFileInfo,
  EvtxRecord,
  FilterOptions,
} from "../lib/types";
import { logger } from "../lib/logger";
import EvtxStorage from "../lib/storage";
import { startFullIngest } from "../lib/fullIngest";

interface UseEvtxLogReturn {
  /* state */
  isLoading: boolean;
  loadingMessage: string;
  records: EvtxRecord[];
  matchedCount: number;
  fileInfo: EvtxFileInfo | null;
  parser: EvtxParser | null;
  dataSource: DuckDbDataSource | null;
  bucketCounts: BucketCounts | null;
  totalRecords: number;
  currentFileId: string | null;
  ingestProgress: number;
  /* actions */
  loadFile: (file: File) => Promise<void>;
}

export function useEvtxLog(filters: FilterOptions): UseEvtxLogReturn {
  const [isLoading, setIsLoading] = useState(false);
  const [loadingMessage, setLoadingMessage] = useState("");
  const [records, setRecords] = useState<EvtxRecord[]>([]);
  const [matchedCount, setMatchedCount] = useState(0);
  const [fileInfo, setFileInfo] = useState<EvtxFileInfo | null>(null);
  const [parser, setParser] = useState<EvtxParser | null>(null);
  const [dataSource, setDataSource] = useState<DuckDbDataSource | null>(null);
  const [bucketCounts, setBucketCounts] = useState<BucketCounts | null>(null);
  const [totalRecords, setTotalRecords] = useState(0);
  const [currentFileId, setCurrentFileId] = useState<string | null>(null);
  const [ingestProgress, setIngestProgress] = useState(1);

  // Keep a ref to the current ingest so we can cancel it if the user opens a new file
  const ingestAbortRef = useRef<AbortController | null>(null);

  /**
   * Load a new EVTX file, start ingest and update state.
   */
  const loadFile = useCallback(
    async (file: File) => {
      // Reset UI state that will be recomputed for the new file so nothing
      // from the previous log lingers while ingest is in-flight.
      setBucketCounts(null);
      setMatchedCount(0);
      setTotalRecords(0);
      setDataSource(null);

      setIsLoading(true);
      setLoadingMessage("Loading file...");
      logger.info(`Loading file: ${file.name}`);

      try {
        // ----------------- NEW LAZY PATH -----------------
        const reader = await LazyEvtxReader.fromFile(file);
        // For initial loading we still create a placeholder DS after ingest starts.
        setDataSource(new DuckDbDataSource({}));

        // We still parse the first window eagerly so that filters/sidebar can
        // display something immediate (optional – small performance hit).
        const initial = await reader.getWindow({
          chunkIndex: 0,
          start: 0,
          limit: 1000,
        });
        setRecords(initial);
        setMatchedCount(initial.length);

        // Legacy EvtxParser kept for export functionality.
        const evtxParser = new EvtxParser();
        const info = await evtxParser.parseFile(file);
        // Retrieve fileId derived during saveFile inside parser
        const storage = await EvtxStorage.getInstance();
        const fileId = await storage.deriveFileId(file);
        setCurrentFileId(fileId);

        setFileInfo(info);
        setParser(evtxParser);

        // Cancel any previous ingest still in flight
        ingestAbortRef.current?.abort();

        // Kick off background clear+ingest flow without blocking UI
        void (async () => {
          try {
            const { clearLogs } = await import("../lib/duckdb");
            await clearLogs();

            const ctrl = new AbortController();
            ingestAbortRef.current = ctrl;
            setIngestProgress(0);

            await startFullIngest(reader, (pct) => setIngestProgress(pct), {
              signal: ctrl.signal,
            });

            // Retrieve total record count for status bar (no filters)
            try {
              const { countRecords } = await import("../lib/duckdb");
              const total = await countRecords({});
              setTotalRecords(total);
              setMatchedCount(total);
            } catch (err) {
              console.warn("Failed to get totalRecords", err);
            }

            // Now that the DB is fully populated create a fresh data source
            setDataSource(new DuckDbDataSource(filters));
          } catch (e) {
            if (e instanceof DOMException && e.name === "AbortError") return;
            console.warn("Full ingest failed", e);
            setIngestProgress(1);
          }
        })();
      } catch (error) {
        logger.error("Failed to load file via lazy reader", error);
        alert("Failed to parse file. Please check if it's a valid EVTX file.");
      } finally {
        setIsLoading(false);
        setLoadingMessage("");
      }
    },
    [filters]
  );

  // Keep matched count fresh based on DuckDB filters
  useEffect(() => {
    if (ingestProgress < 1) return;
    let active = true;
    (async () => {
      try {
        const { countRecords } = await import("../lib/duckdb");
        const n = await countRecords(filters);
        if (active) setMatchedCount(n);
      } catch (err) {
        console.warn("Failed to count records", err);
      }
    })();
    return () => {
      active = false;
    };
  }, [filters, ingestProgress]);

  // Update bucket counts from DuckDB when filters or DB ingestion change
  useEffect(() => {
    if (ingestProgress < 1) return;
    let active = true;
    (async () => {
      try {
        const { initDuckDB, getFacetCounts } = await import("../lib/duckdb");
        await initDuckDB();
        const counts = await getFacetCounts(filters);
        if (active) setBucketCounts(counts);
      } catch (err) {
        console.warn("Failed to get DuckDB facet counts", err);
      }
    })();
    return () => {
      active = false;
    };
  }, [filters, ingestProgress]);

  // Recreate data source whenever filters change (after ingest ready)
  useEffect(() => {
    if (ingestProgress < 1) return;
    logger.debug("build new DuckDbDataSource", { filters });
    setDataSource(new DuckDbDataSource(filters));
  }, [filters, ingestProgress]);

  return {
    isLoading,
    loadingMessage,
    records,
    matchedCount,
    fileInfo,
    parser,
    dataSource,
    bucketCounts,
    totalRecords,
    currentFileId,
    ingestProgress,
    loadFile,
  };
}
