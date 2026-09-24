import { useMemo } from "react";
import type { TableColumn } from "../lib/types";
import { useColumnsState, useAllColumnsState, useGlobalDispatch } from "../state/store";
import { setColumns, addColumn, removeColumn, resizeColumn } from "../state/columns/columnsSlice";

export function useColumns() {
  const columns = useColumnsState();
  const allColumns = useAllColumnsState();
  const dispatch = useGlobalDispatch();
  const actions = useMemo(
    () => ({
      setColumns: (next: TableColumn[]) => dispatch(setColumns(next)),
      addColumn: (column: TableColumn) => dispatch(addColumn(column)),
      removeColumn: (id: string) => dispatch(removeColumn(id)),
      resizeColumn: (id: string, width: number) => dispatch(resizeColumn(id, width)),
    }),
    [dispatch],
  );
  return { columns, allColumns, ...actions };
}
