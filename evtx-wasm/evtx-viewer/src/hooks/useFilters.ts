import { useCallback } from "react";
import type { FilterOptions } from "../lib/types";
import { useFiltersState, useGlobalDispatch } from "../state/store";
import {
  setFilters as setFiltersAction,
  updateFilters as updateFiltersAction,
  clearFilters as clearFiltersAction,
} from "../state/filters/filtersSlice";

/**
 * Global reducer-backed filters state using central store.
 * Mirrors React.useState signature to ease migration.
 */
export function useFilters() {
  const filters = useFiltersState();
  const dispatch = useGlobalDispatch();

  type UpdaterFn = (prev: FilterOptions) => FilterOptions;

  const setFilters = useCallback(
    (next: FilterOptions | UpdaterFn) => {
      const payload =
        typeof next === "function" ? (next as UpdaterFn)(filters) : next;
      dispatch(setFiltersAction(payload));
    },
    [dispatch, filters]
  );

  const updateFilters = useCallback(
    (patch: Partial<FilterOptions>) => dispatch(updateFiltersAction(patch)),
    [dispatch]
  );

  const clearFilters = useCallback(
    () => dispatch(clearFiltersAction()),
    [dispatch]
  );

  return { filters, setFilters, updateFilters, clearFilters } as const;
}
