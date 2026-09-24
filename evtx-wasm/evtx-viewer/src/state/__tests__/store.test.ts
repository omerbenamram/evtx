import { expect, it } from "vitest";
import { globalInitialState, rootReducer } from "../rootReducer";
import { setFilters, updateFilters, clearFilters } from "../filters/filtersSlice";

it("composes queued filter updates against current state and keeps columns on clear", () => {
  let state = rootReducer(globalInitialState, setFilters({ searchQuery: "event_id:4624" }));
  const originalColumns = state.columns;
  const addHost = updateFilters((filters) => ({
    ...filters,
    searchQuery: `${filters.searchQuery} computer:HOST`,
  }));
  const setRange = updateFilters((filters) => ({
    ...filters,
    timeRange: { start: new Date(0), end: new Date(1) },
  }));
  state = rootReducer(rootReducer(state, addHost), setRange);
  expect(state.filters).toEqual({
    searchQuery: "event_id:4624 computer:HOST",
    timeRange: { start: new Date(0), end: new Date(1) },
  });
  state = rootReducer(state, clearFilters());
  expect(state.filters).toEqual({});
  expect(state.columns).toBe(originalColumns);
  expect(state.evtx).toBe(globalInitialState.evtx);
});
