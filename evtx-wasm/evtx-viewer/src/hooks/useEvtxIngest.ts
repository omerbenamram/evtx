import { useCallback, useEffect, useRef } from "react";
import { EvtxWorkerReader } from "../lib/workerReader";
import { useGlobalDispatch } from "../state/store";
import { evtxInitialState, updateEvtxMeta } from "../state/evtx/evtxSlice";
import EvtxStorage, { fileIdFor } from "../lib/storage";
import { startFullIngest } from "../lib/fullIngest";
import { clearLogs, initDuckDB, countRecords } from "../lib/duckdb";
import { errorMessage } from "../lib/types";

export function useEvtxIngest() {
  const dispatch = useGlobalDispatch();
  const active = useRef<AbortController | null>(null);
  const pending = useRef<Promise<void>>(Promise.resolve());
  const currentFile = useRef<File | null>(null);

  useEffect(() => () => active.current?.abort(), []);

  const cancel = useCallback(() => active.current?.abort(), []);

  const loadFile = useCallback(
    (file: File): Promise<void> => {
      active.current?.abort();
      const controller = new AbortController();
      active.current = controller;
      currentFile.current = file;
      const { signal } = controller;
      const previous = pending.current;

      const task = (async () => {
        // An already-submitted database insert must finish before the next clear.
        await previous;
        if (signal.aborted) return;
        dispatch(
          updateEvtxMeta({ ...evtxInitialState, isLoading: true, loadingMessage: "Opening log…" }),
        );
        let reader: EvtxWorkerReader | undefined;
        let hasTable = false;
        const warnings = new Set<string>();
        const warn = (message: string) => {
          // ponytail: retain 100 diagnostics; add downloadable diagnostics if full reports are needed.
          if (warnings.size < 100) warnings.add(message);
          else warnings.add("Additional import warnings omitted after the first 100.");
        };
        try {
          reader = new EvtxWorkerReader(signal);
          const [opened] = await Promise.all([reader.open(file), initDuckDB()]);
          signal.throwIfAborted();
          await clearLogs();
          hasTable = true;
          signal.throwIfAborted();
          dispatch(
            updateEvtxMeta({
              fileInfo: opened.info,
              currentFileId: fileIdFor(file),
              loadingMessage: "Importing events…",
            }),
          );
          let lastPublished = 0;
          await startFullIngest(
            reader,
            opened.info.totalChunks,
            signal,
            (progress, records) => {
              const now = performance.now();
              if (lastPublished && now - lastPublished < 150 && progress !== 1) return;
              lastPublished = now;
              dispatch(
                updateEvtxMeta({
                  totalRecords: records,
                  ingestProgress: progress,
                  warnings: [...warnings],
                }),
              );
            },
            warn,
          );
          signal.throwIfAborted();
          // The worker owns the only parser and releases its file memory here.
          reader.close();
          dispatch(updateEvtxMeta({ loadingMessage: "Saving to Recent Logs…" }));
          // Only a completed import is kept in Recent Logs.
          await EvtxStorage.getInstance()
            .then((storage) => storage.saveFile(file, opened.info.totalChunks))
            .catch((cause: unknown) =>
              warn(`This log could not be saved in Recent Logs: ${errorMessage(cause)}`),
            );
        } catch (error) {
          if (active.current !== controller) return;
          if (signal.aborted) dispatch(updateEvtxMeta({ cancelled: true }));
          else dispatch(updateEvtxMeta({ loadError: errorMessage(error) }));
        } finally {
          reader?.close();
          if (active.current === controller) {
            if (hasTable) {
              try {
                const totalRecords = await countRecords({});
                dispatch(updateEvtxMeta({ totalRecords }));
              } catch (error) {
                dispatch(updateEvtxMeta({ loadError: errorMessage(error) }));
              }
            }
            dispatch(
              updateEvtxMeta({
                isLoading: false,
                loadingMessage: "",
                warnings: [...warnings],
              }),
            );
          }
        }
      })();
      pending.current = task;
      return task;
    },
    [dispatch],
  );

  const reload = useCallback(
    () => (currentFile.current ? loadFile(currentFile.current) : Promise.resolve()),
    [loadFile],
  );
  return { loadFile, cancel, reload, currentFile };
}
