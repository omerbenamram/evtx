import type { EvtxFileInfo } from "../../lib/types";

export interface EvtxMetaState {
  isLoading: boolean;
  loadingMessage: string;
  matchedCount: number;
  totalRecords: number;
  ingestProgress: number;
  fileInfo: EvtxFileInfo | null;
  currentFileId: string | null;
  loadError: string | null;
  warnings: string[];
  cancelled: boolean;
}

export const evtxInitialState: EvtxMetaState = {
  isLoading: false,
  loadingMessage: "",
  matchedCount: 0,
  totalRecords: 0,
  ingestProgress: 0,
  fileInfo: null,
  currentFileId: null,
  loadError: null,
  warnings: [],
  cancelled: false,
};

export type EvtxAction = { type: "evtx/UPDATE"; payload: Partial<EvtxMetaState> };

export const updateEvtxMeta = (payload: Partial<EvtxMetaState>): EvtxAction => ({
  type: "evtx/UPDATE",
  payload,
});
