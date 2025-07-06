import { useEffect } from "react";
import { useGlobalDispatch } from "./store";
import { setFilters } from "./filters/filtersSlice";
import { setColumns } from "./columns/columnsSlice";
import { getDefaultColumns } from "../lib/columns";
import type { FilterOptions } from "../lib/types";

/**
 * One-time bootstrap component. Mount it once under GlobalProvider
 * to seed default columns and optional initial filters.
 */
export const StoreBootstrap: React.FC<{ initialFilters?: FilterOptions }> = ({
  initialFilters = {},
}) => {
  const dispatch = useGlobalDispatch();

  useEffect(() => {
    dispatch(setFilters(initialFilters));
    dispatch(setColumns(getDefaultColumns()));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return null;
};
