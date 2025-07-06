import { useEffect } from "react";
import type { EvtxParser } from "../lib/parser";
import type { DuckDbDataSource } from "../lib/duckDbDataSource";
import type { EvtxFileInfo, EvtxRecord } from "../lib/types";
import {
  useFiltersState,
  useColumnsState,
  useGlobalDispatch,
  useIngestState,
  useEvtxMetaState,
} from "../state/store";
import { updateEvtxMeta } from "../state/evtx/evtxSlice";
// ingest slice actions are handled inside useEvtxIngest
import { setActiveColumns } from "../lib/duckdb";
import { useEvtxIngest } from "./useEvtxIngest";

interface UseEvtxLogReturn {
  /* state */
  isLoading: boolean;
  loadingMessage: string;
  records: EvtxRecord[];
  matchedCount: number;
  fileInfo: EvtxFileInfo | null;
  parser: EvtxParser | null;
  dataSource: DuckDbDataSource | null;
  totalRecords: number;
  currentFileId: string | null;
  ingestProgress: number;
  /* actions */
  loadFile: (file: File) => Promise<void>;
}

export function useEvtxLog(): UseEvtxLogReturn {
  const filters = useFiltersState();
  const columns = useColumnsState();
  const dispatch = useGlobalDispatch();
  const ingest = useIngestState();
  const evtxMeta = useEvtxMetaState();
  const {
    isLoading,
    loadingMessage,
    matchedCount,
    totalRecords: metaTotalRecords,
    currentFileId: metaCurrentFileId,
  } = evtxMeta;

  const { records, parser, fileInfo, dataSource, updateDataSource, loadFile } =
    useEvtxIngest();

  const { totalRecords, currentFileId, progress: ingestProgress } = ingest;

  // Keep active columns in duckdb helper (unchanged)
  useEffect(() => {
    setActiveColumns(columns);
  }, [columns]);

  // Keep matched count fresh based on DuckDB filters
  useEffect(() => {
    if (ingestProgress < 1) return;
    let active = true;
    (async () => {
      try {
        const { countRecords } = await import("../lib/duckdb");
        const n = await countRecords(filters);
        if (active) dispatch(updateEvtxMeta({ matchedCount: n }));
      } catch (err) {
        console.warn("Failed to count records", err);
      }
    })();
    return () => {
      active = false;
    };
  }, [filters, columns, ingestProgress]);

  // Recreate data source whenever filters change (after ingest ready)
  useEffect(() => {
    if (ingestProgress < 1) return;
    import("../lib/duckDbDataSource").then(({ DuckDbDataSource }) => {
      updateDataSource(new DuckDbDataSource(filters, columns));
    });
  }, [filters, columns, ingestProgress]);

  return {
    isLoading,
    loadingMessage,
    records,
    matchedCount,
    fileInfo,
    parser,
    dataSource,
    totalRecords: metaTotalRecords || totalRecords,
    currentFileId: metaCurrentFileId || currentFileId,
    ingestProgress,
    loadFile,
  } as const;
}
