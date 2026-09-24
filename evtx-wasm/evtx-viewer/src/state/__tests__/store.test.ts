import { expect, it } from "vitest";
import { globalInitialState, rootReducer } from "../rootReducer";
import { setFilters, updateFilters, clearFilters } from "../filters/filtersSlice";

it("composes queued filter updates against current state and keeps columns on clear", () => {
  let state = rootReducer(globalInitialState, setFilters({ include: { eventId: ["4624"] } }));
  const originalColumns = state.columns;
  const addHost = updateFilters((filters) => ({
    ...filters,
    include: { ...filters.include, computer: ["HOST"] },
  }));
  const excludeProvider = updateFilters((filters) => ({
    ...filters,
    exclude: { provider: ["Security"] },
  }));
  state = rootReducer(rootReducer(state, addHost), excludeProvider);
  expect(state.filters).toEqual({
    include: { eventId: ["4624"], computer: ["HOST"] },
    exclude: { provider: ["Security"] },
  });
  state = rootReducer(state, clearFilters());
  expect(state.filters).toEqual({});
  expect(state.columns).toBe(originalColumns);
  expect(state.evtx).toBe(globalInitialState.evtx);
});
