import { useCallback, useRef, useState } from "react";
import { LazyEvtxReader } from "../lib/lazyReader";
import { EvtxParser } from "../lib/parser";
import { DuckDbDataSource } from "../lib/duckDbDataSource";
import type { EvtxFileInfo, EvtxRecord } from "../lib/types";
import {
  useFiltersState,
  useColumnsState,
  useGlobalDispatch,
} from "../state/store";
import { updateEvtxMeta } from "../state/evtx/evtxSlice";
import {
  setIngestProgress as dispatchIngestProgress,
  setIngestTotal,
  setIngestFileId,
} from "../state/ingest/ingestSlice";
import { logger } from "../lib/logger";
import EvtxStorage from "../lib/storage";
import { startFullIngest } from "../lib/fullIngest";

interface UseEvtxIngestReturn {
  records: EvtxRecord[];
  parser: EvtxParser | null;
  fileInfo: EvtxFileInfo | null;
  dataSource: DuckDbDataSource | null;
  updateDataSource: (ds: DuckDbDataSource | null) => void;
  loadFile: (file: File) => Promise<void>;
}

export function useEvtxIngest(): UseEvtxIngestReturn {
  const filters = useFiltersState();
  const columns = useColumnsState();
  const dispatch = useGlobalDispatch();

  const [records, setRecords] = useState<EvtxRecord[]>([]);
  const [parser, setParser] = useState<EvtxParser | null>(null);
  const [fileInfo, setFileInfo] = useState<EvtxFileInfo | null>(null);
  const [dataSource, setDataSource] = useState<DuckDbDataSource | null>(null);

  const ingestAbortRef = useRef<AbortController | null>(null);

  const loadFile = useCallback(
    async (file: File) => {
      dispatch(updateEvtxMeta({ matchedCount: 0 }));
      dispatch(setIngestTotal(0));
      setDataSource(null);
      dispatch(
        updateEvtxMeta({ isLoading: true, loadingMessage: "Loading file..." })
      );

      try {
        const reader = await LazyEvtxReader.fromFile(file);
        setDataSource(new DuckDbDataSource({}, columns));

        const initial = await reader.getWindow({
          chunkIndex: 0,
          start: 0,
          limit: 1000,
        });
        setRecords(initial);
        dispatch(updateEvtxMeta({ matchedCount: initial.length }));

        const evtxParser = new EvtxParser();
        const info = await evtxParser.parseFile(file);
        const storage = await EvtxStorage.getInstance();
        const fileId = await storage.deriveFileId(file);
        dispatch(setIngestFileId(fileId));
        dispatch(updateEvtxMeta({ currentFileId: fileId }));

        setFileInfo(info);
        // Propagate file info to global state so components like StatusBar can show it
        dispatch(updateEvtxMeta({ fileInfo: info }));
        setParser(evtxParser);

        ingestAbortRef.current?.abort();

        void (async () => {
          try {
            const { clearLogs, countRecords } = await import("../lib/duckdb");
            await clearLogs();
            const ctrl = new AbortController();
            ingestAbortRef.current = ctrl;
            dispatch(dispatchIngestProgress(0));

            await startFullIngest(
              reader,
              (pct) => dispatch(dispatchIngestProgress(pct)),
              { signal: ctrl.signal }
            );

            try {
              const total = await countRecords({});
              dispatch(setIngestTotal(total));
              dispatch(
                updateEvtxMeta({ totalRecords: total, matchedCount: total })
              );
            } catch (err) {
              console.warn("Failed to get totalRecords", err);
            }

            setDataSource(new DuckDbDataSource(filters, columns));
          } catch (e) {
            if (e instanceof DOMException && e.name === "AbortError") return;
            console.warn("Full ingest failed", e);
            dispatch(dispatchIngestProgress(1));
          }
        })();
      } catch (err) {
        logger.error("Failed to load file via lazy reader", err);
        alert("Failed to parse file. Please check if it's a valid EVTX file.");
      } finally {
        dispatch(updateEvtxMeta({ isLoading: false, loadingMessage: "" }));
      }
    },
    [columns, filters, dispatch]
  );

  return {
    records,
    parser,
    fileInfo,
    dataSource,
    updateDataSource: setDataSource,
    loadFile,
  } as const;
}
