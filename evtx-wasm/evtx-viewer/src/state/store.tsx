import {
  createContext,
  useContext,
  useMemo,
  useReducer,
  type Context,
  type Dispatch,
  type PropsWithChildren,
} from "react";
import {
  rootReducer,
  globalInitialState,
  type GlobalState,
  type GlobalAction,
} from "./rootReducer";

const FiltersContext = createContext<GlobalState["filters"] | null>(null);
const ColumnsContext = createContext<GlobalState["columns"] | null>(null);
const EventsContext = createContext<GlobalState["evtx"] | null>(null);
const DispatchContext = createContext<Dispatch<GlobalAction> | null>(null);

export function GlobalProvider({ children }: PropsWithChildren) {
  const [state, dispatch] = useReducer(rootReducer, globalInitialState);
  return (
    <DispatchContext.Provider value={dispatch}>
      <FiltersContext.Provider value={state.filters}>
        <ColumnsContext.Provider value={state.columns}>
          <EventsContext.Provider value={state.evtx}>{children}</EventsContext.Provider>
        </ColumnsContext.Provider>
      </FiltersContext.Provider>
    </DispatchContext.Provider>
  );
}

function useRequiredContext<T>(context: Context<T | null>): T {
  const value = useContext(context);
  if (value === null) throw new Error("Store hooks require GlobalProvider");
  return value;
}

export const useGlobalDispatch = () => useRequiredContext(DispatchContext);
export const useFiltersState = () => useRequiredContext(FiltersContext);
export const useAllColumnsState = () => useRequiredContext(ColumnsContext);
export function useColumnsState() {
  const allColumns = useAllColumnsState();
  return useMemo(() => allColumns.filter((column) => !column.hidden), [allColumns]);
}
export const useEvtxMetaState = () => useRequiredContext(EventsContext);
