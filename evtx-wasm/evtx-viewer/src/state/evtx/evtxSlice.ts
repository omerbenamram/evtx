import type { EvtxFileInfo } from "../../lib/types";

// ---------------- Types & State ----------------
export interface EvtxMetaState {
  isLoading: boolean;
  loadingMessage: string;
  matchedCount: number;
  totalRecords: number;
  fileInfo: EvtxFileInfo | null;
  currentFileId: string | null;
}

export const evtxInitialState: EvtxMetaState = {
  isLoading: false,
  loadingMessage: "",
  matchedCount: 0,
  totalRecords: 0,
  fileInfo: null,
  currentFileId: null,
};

// ---------------- Action Types ----------------
export type EvtxAction = {
  type: "evtx/UPDATE";
  payload: Partial<EvtxMetaState>;
};

// ---------------- Reducer ----------------
export function evtxReducer(
  state: EvtxMetaState = evtxInitialState,
  action: EvtxAction
): EvtxMetaState {
  switch (action.type) {
    case "evtx/UPDATE":
      return { ...state, ...action.payload };
    default:
      return state;
  }
}

// ---------------- Action Creators ----------------
export const updateEvtxMeta = (
  payload: Partial<EvtxMetaState>
): EvtxAction => ({
  type: "evtx/UPDATE",
  payload,
});
