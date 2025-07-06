import { filtersReducer, filtersInitialState } from "./filters/filtersSlice";
import { columnsReducer, columnsInitialState } from "./columns/columnsSlice";
import { ingestReducer, ingestInitialState } from "./ingest/ingestSlice";
import type { IngestState } from "./ingest/ingestSlice";
import { evtxReducer, evtxInitialState } from "./evtx/evtxSlice";
import type { EvtxMetaState, EvtxAction } from "./evtx/evtxSlice";

import type { FiltersAction } from "./filters/filtersSlice";
import type { ColumnsAction } from "./columns/columnsSlice";
import type { IngestAction } from "./ingest/ingestSlice";

import type { FilterOptions, TableColumn } from "../lib/types";

// ----------------- Global State & Actions -----------------

export interface GlobalState {
  filters: FilterOptions;
  columns: TableColumn[];
  ingest: IngestState;
  evtx: EvtxMetaState;
}

export type GlobalAction =
  | FiltersAction
  | ColumnsAction
  | IngestAction
  | EvtxAction;

export const globalInitialState: GlobalState = {
  filters: filtersInitialState,
  columns: columnsInitialState,
  ingest: ingestInitialState,
  evtx: evtxInitialState,
};

// Root reducer delegates to slice reducers.
export function rootReducer(
  state: GlobalState = globalInitialState,
  action: GlobalAction
): GlobalState {
  return {
    filters: filtersReducer(state.filters, action as FiltersAction),
    columns: columnsReducer(state.columns, action as ColumnsAction),
    ingest: ingestReducer(state.ingest, action as IngestAction),
    evtx: evtxReducer(state.evtx, action as EvtxAction),
  };
}
