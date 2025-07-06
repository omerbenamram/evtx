import type { TableColumn } from "../../lib/types";

// ---------------- Initial State ----------------
export const columnsInitialState: TableColumn[] = [];

// ---------------- Action Types ----------------
export type ColumnsAction =
  | { type: "columns/SET"; payload: TableColumn[] }
  | { type: "columns/ADD"; payload: TableColumn }
  | { type: "columns/REMOVE"; payload: string };

// ---------------- Reducer ----------------
export function columnsReducer(
  state: TableColumn[] = columnsInitialState,
  action: ColumnsAction
): TableColumn[] {
  switch (action.type) {
    case "columns/SET":
      return action.payload;
    case "columns/ADD":
      if (state.some((c) => c.id === action.payload.id)) return state;
      return [...state, action.payload];
    case "columns/REMOVE":
      return state.filter((c) => c.id !== action.payload);
    default:
      return state;
  }
}

// ---------------- Action Creators ----------------
export const setColumns = (payload: TableColumn[]): ColumnsAction => ({
  type: "columns/SET",
  payload,
});

export const addColumn = (payload: TableColumn): ColumnsAction => ({
  type: "columns/ADD",
  payload,
});

export const removeColumn = (id: string): ColumnsAction => ({
  type: "columns/REMOVE",
  payload: id,
});
