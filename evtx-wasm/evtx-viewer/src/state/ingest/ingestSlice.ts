// Simple ingest slice managing progress & totals

export interface IngestState {
  progress: number; // 0..1
  totalRecords: number;
  currentFileId: string | null;
}

export const ingestInitialState: IngestState = {
  progress: 1,
  totalRecords: 0,
  currentFileId: null,
};

export type IngestAction =
  | { type: "ingest/SET_PROGRESS"; payload: number }
  | { type: "ingest/SET_TOTAL"; payload: number }
  | { type: "ingest/SET_FILE_ID"; payload: string | null };

export function ingestReducer(
  state: IngestState = ingestInitialState,
  action: IngestAction
): IngestState {
  switch (action.type) {
    case "ingest/SET_PROGRESS":
      return { ...state, progress: action.payload };
    case "ingest/SET_TOTAL":
      return { ...state, totalRecords: action.payload };
    case "ingest/SET_FILE_ID":
      return { ...state, currentFileId: action.payload };
    default:
      return state;
  }
}

export const setIngestProgress = (pct: number): IngestAction => ({
  type: "ingest/SET_PROGRESS",
  payload: pct,
});
export const setIngestTotal = (total: number): IngestAction => ({
  type: "ingest/SET_TOTAL",
  payload: total,
});
export const setIngestFileId = (id: string | null): IngestAction => ({
  type: "ingest/SET_FILE_ID",
  payload: id,
});
