import type { FilterOptions } from "../../lib/types";

// ---------------- Initial State ----------------
export const filtersInitialState: FilterOptions = {};

// ---------------- Action Types ----------------
export type FiltersAction =
  | { type: "filters/SET"; payload: FilterOptions }
  | { type: "filters/UPDATE"; payload: Partial<FilterOptions> }
  | { type: "filters/CLEAR" };

// ---------------- Reducer ----------------
export function filtersReducer(
  state: FilterOptions = filtersInitialState,
  action: FiltersAction
): FilterOptions {
  switch (action.type) {
    case "filters/SET":
      return action.payload;
    case "filters/UPDATE":
      return { ...state, ...action.payload };
    case "filters/CLEAR":
      return {};
    default:
      return state;
  }
}

// ---------------- Action Creators ----------------
export const setFilters = (payload: FilterOptions): FiltersAction => ({
  type: "filters/SET",
  payload,
});

export const updateFilters = (
  payload: Partial<FilterOptions>
): FiltersAction => ({
  type: "filters/UPDATE",
  payload,
});

export const clearFilters = (): FiltersAction => ({
  type: "filters/CLEAR",
});
