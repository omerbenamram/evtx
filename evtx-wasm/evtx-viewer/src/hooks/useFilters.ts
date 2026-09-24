import { useMemo } from "react";
import type { FilterOptions } from "../lib/types";
import { useFiltersState, useGlobalDispatch } from "../state/store";
import {
  setFilters,
  updateFilters,
  clearFilters,
  type FilterUpdate,
} from "../state/filters/filtersSlice";

export function useFilters() {
  const filters = useFiltersState();
  const dispatch = useGlobalDispatch();
  const actions = useMemo(
    () => ({
      setFilters: (next: FilterOptions) => dispatch(setFilters(next)),
      updateFilters: (updater: FilterUpdate) => dispatch(updateFilters(updater)),
      clearFilters: () => dispatch(clearFilters()),
    }),
    [dispatch],
  );
  return { filters, ...actions };
}
