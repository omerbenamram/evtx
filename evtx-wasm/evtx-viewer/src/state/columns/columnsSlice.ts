import type { TableColumn } from "../../lib/types";

export const MIN_COLUMN_WIDTH = 72;
export const MAX_COLUMN_WIDTH = 2000;

const clampColumnWidth = (width: number): number =>
  Math.max(MIN_COLUMN_WIDTH, Math.min(MAX_COLUMN_WIDTH, Math.round(width)));

export type ColumnsAction =
  | { type: "columns/SET"; payload: TableColumn[] }
  | { type: "columns/ADD"; payload: TableColumn }
  | { type: "columns/REMOVE"; payload: string }
  | { type: "columns/RESIZE"; payload: { id: string; width: number } };

export function columnsReducer(state: TableColumn[], action: ColumnsAction): TableColumn[] {
  switch (action.type) {
    case "columns/SET": {
      if (!action.payload.length) return state;
      const visibleIds = new Set(action.payload.map((column) => column.id));
      return [
        ...action.payload.map((column) => ({ ...column, hidden: false })),
        ...state
          .filter((column) => !visibleIds.has(column.id))
          .map((column) => ({ ...column, hidden: true })),
      ];
    }
    case "columns/ADD": {
      const existing = state.find((column) => column.id === action.payload.id);
      if (!existing) return [...state, { ...action.payload, hidden: false }];
      return existing.hidden
        ? state.map((column) => (column.id === existing.id ? { ...column, hidden: false } : column))
        : state;
    }
    case "columns/REMOVE":
      if (state.filter((column) => !column.hidden).length <= 1) return state;
      return state.map((column) =>
        column.id === action.payload ? { ...column, hidden: true } : column,
      );
    case "columns/RESIZE": {
      if (!Number.isFinite(action.payload.width)) return state;
      const width = clampColumnWidth(action.payload.width);
      if (!state.some((column) => column.id === action.payload.id && column.width !== width))
        return state;
      return state.map((column) =>
        column.id === action.payload.id ? { ...column, width } : column,
      );
    }
  }
}

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

export const resizeColumn = (id: string, width: number): ColumnsAction => ({
  type: "columns/RESIZE",
  payload: { id, width },
});
