import type { FiltersAction } from "./filters/filtersSlice";
import { columnsReducer, type ColumnsAction } from "./columns/columnsSlice";
import { evtxInitialState, type EvtxMetaState, type EvtxAction } from "./evtx/evtxSlice";
import { getDefaultColumns } from "../lib/columns";
import type { FilterOptions, TableColumn } from "../lib/types";

export interface GlobalState {
  filters: FilterOptions;
  columns: TableColumn[];
  evtx: EvtxMetaState;
}

export type GlobalAction = FiltersAction | ColumnsAction | EvtxAction;

export const globalInitialState: GlobalState = {
  filters: {},
  columns: getDefaultColumns(),
  evtx: evtxInitialState,
};

export function rootReducer(state: GlobalState, action: GlobalAction): GlobalState {
  switch (action.type) {
    case "filters/SET":
      return { ...state, filters: action.payload };
    case "filters/UPDATE":
      return { ...state, filters: action.payload(state.filters) };
    case "filters/CLEAR":
      return { ...state, filters: {} };
    case "columns/SET":
    case "columns/ADD":
    case "columns/REMOVE":
    case "columns/RESIZE":
      return { ...state, columns: columnsReducer(state.columns, action) };
    case "evtx/UPDATE":
      return { ...state, evtx: { ...state.evtx, ...action.payload } };
  }
}
