import { useMemo } from "react";
import { DuckDbDataSource } from "../lib/duckDbDataSource";
import { useFiltersState, useColumnsState, useEvtxMetaState } from "../state/store";
import { useEvtxIngest } from "./useEvtxIngest";

/** Builds a query snapshot per store change; the table decides when to adopt and count it. */
export function useEvtxLog() {
  const filters = useFiltersState();
  const columns = useColumnsState();
  const meta = useEvtxMetaState();
  const actions = useEvtxIngest();
  const { fileInfo, totalRecords } = meta;

  const dataSource = useMemo(
    () => (fileInfo ? new DuckDbDataSource(filters, columns, null, undefined, totalRecords) : null),
    [fileInfo, totalRecords, filters, columns],
  );

  return { ...meta, ...actions, dataSource };
}
