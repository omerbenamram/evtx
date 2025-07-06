import { useCallback } from "react";
import type { TableColumn } from "../lib/types";
import { useColumnsState, useGlobalDispatch } from "../state/store";
import {
  setColumns as setColumnsAction,
  addColumn as addColumnAction,
  removeColumn as removeColumnAction,
} from "../state/columns/columnsSlice";

/**
 * Global reducer-backed columns state.
 * Provides the same API shape as React.useState for drop-in replacement.
 */
export function useColumns() {
  const columns = useColumnsState();
  const dispatch = useGlobalDispatch();

  type UpdaterFn = (prev: TableColumn[]) => TableColumn[];

  const setColumns = useCallback(
    (next: TableColumn[] | UpdaterFn) => {
      const payload =
        typeof next === "function" ? (next as UpdaterFn)(columns) : next;
      dispatch(setColumnsAction(payload));
    },
    [dispatch, columns]
  );

  const addColumn = useCallback(
    (col: TableColumn) => dispatch(addColumnAction(col)),
    [dispatch]
  );

  const removeColumn = useCallback(
    (id: string) => dispatch(removeColumnAction(id)),
    [dispatch]
  );

  return { columns, setColumns, addColumn, removeColumn } as const;
}
