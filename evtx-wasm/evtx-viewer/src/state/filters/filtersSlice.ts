import type { FilterOptions } from "../../lib/types";

export type FilterUpdate = (current: FilterOptions) => FilterOptions;
export type FiltersAction =
  | { type: "filters/SET"; payload: FilterOptions }
  | { type: "filters/UPDATE"; payload: FilterUpdate }
  | { type: "filters/CLEAR" };

export const setFilters = (payload: FilterOptions): FiltersAction => ({
  type: "filters/SET",
  payload,
});
export const updateFilters = (payload: FilterUpdate): FiltersAction => ({
  type: "filters/UPDATE",
  payload,
});
export const clearFilters = (): FiltersAction => ({ type: "filters/CLEAR" });
