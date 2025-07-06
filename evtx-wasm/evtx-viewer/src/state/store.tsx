import React, { createContext, useContext, useReducer } from "react";
import type { Dispatch, ReactNode } from "react";
import { rootReducer, globalInitialState } from "./rootReducer";
import type { GlobalState, GlobalAction } from "./rootReducer";

// Separate contexts for state and dispatch to minimise re-renders
const StateCtx = createContext<GlobalState | undefined>(undefined);
const DispatchCtx = createContext<Dispatch<GlobalAction> | undefined>(
  undefined
);

interface ProviderProps {
  children: ReactNode;
}

export const GlobalProvider: React.FC<ProviderProps> = ({ children }) => {
  const [state, dispatch] = useReducer(rootReducer, globalInitialState);
  return (
    <StateCtx.Provider value={state}>
      <DispatchCtx.Provider value={dispatch}>{children}</DispatchCtx.Provider>
    </StateCtx.Provider>
  );
};

// ---------------- Selector + Dispatch hooks ----------------

// eslint-disable-next-line react-refresh/only-export-components
export function useGlobalState<T>(selector: (s: GlobalState) => T): T {
  const state = useContext(StateCtx);
  if (!state)
    throw new Error("useGlobalState must be used within GlobalProvider");
  return selector(state);
}

// eslint-disable-next-line react-refresh/only-export-components
export function useGlobalDispatch(): Dispatch<GlobalAction> {
  const dispatch = useContext(DispatchCtx);
  if (!dispatch)
    throw new Error("useGlobalDispatch must be used within GlobalProvider");
  return dispatch;
}

// Convenience slice hooks (maintain API parity)
// eslint-disable-next-line react-refresh/only-export-components
export const useFiltersState = () => useGlobalState((s) => s.filters);
// eslint-disable-next-line react-refresh/only-export-components
export const useColumnsState = () => useGlobalState((s) => s.columns);
// eslint-disable-next-line react-refresh/only-export-components
export const useIngestState = () => useGlobalState((s) => s.ingest);
// eslint-disable-next-line react-refresh/only-export-components
export const useEvtxMetaState = () => useGlobalState((s) => s.evtx);
