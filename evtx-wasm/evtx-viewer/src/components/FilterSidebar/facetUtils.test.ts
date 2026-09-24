import { expect, it } from "vitest";
import { formatEventTime } from "../../lib/timeZone";
import { levelName } from "../../lib/types";
import {
  buildFacetConfigs,
  clearFacet,
  facetValues,
  formatFacetValue,
  isFiltered,
  toggleFacet,
} from "./facetUtils";

it("toggles one column's values without dropping other filters", () => {
  const initial = { searchQuery: "computer:HOST", include: { eventId: ["4624"] } };
  const added = toggleFacet(initial, "level", "2");
  expect(added).toEqual({ ...initial, include: { eventId: ["4624"], level: ["2"] } });
  expect(facetValues(added, "level")).toEqual(["2"]);
  expect(toggleFacet(added, "level", "2")).toEqual({
    ...initial,
    include: { eventId: ["4624"], level: [] },
  });
  expect(clearFacet(added, "eventId")).toEqual({
    ...initial,
    include: { level: ["2"] },
    exclude: {},
  });
});

it("forces includes and excludes idempotently and keeps missing values distinct from zero", () => {
  const excluded = toggleFacet({}, "provider", "Security", "exclude", true);
  expect(excluded).toEqual({ exclude: { provider: ["Security"] } });
  expect(toggleFacet(excluded, "provider", "Security", "exclude", true)).toBe(excluded);
  expect(toggleFacet(excluded, "provider", "Other", "exclude", false)).toBe(excluded);
  expect(facetValues(excluded, "provider")).toEqual([]);
  expect(isFiltered(excluded, "provider")).toBe(true);
  // Including a value takes it out of the exclusions; clearing drops both.
  const included = toggleFacet(excluded, "provider", "Security", "include", true);
  expect(included).toEqual({ include: { provider: ["Security"] }, exclude: { provider: [] } });
  expect(isFiltered(clearFacet(excluded, "provider"), "provider")).toBe(false);

  const selected = toggleFacet({ include: { level: ["0"] } }, "level", "");
  expect(selected).toEqual({ include: { level: ["0", ""] } });
  expect(toggleFacet(selected, "level", "")).toEqual({ include: { level: ["0"] } });
});

it("labels unset options consistently and keeps millisecond timestamps distinct", () => {
  const [level, time] = buildFacetConfigs([]);
  const user = { id: "user", label: "User" };
  expect(formatFacetValue(level, "")).toBe("(Not set)");
  expect(formatFacetValue(level, "0")).toBe(levelName(0));
  expect(formatFacetValue(user, "")).toBe("(Not set)");
  expect(formatFacetValue(time, "2026-09-24T12:34:56.123000Z")).toBe(
    formatEventTime("2026-09-24T12:34:56.123Z"),
  );
  expect(formatEventTime("2026-09-24T12:34:56.123Z")).not.toBe(
    formatEventTime("2026-09-24T12:34:56.124Z"),
  );
  expect(formatEventTime("2026-09-24T12:34:56.123Z")).not.toBe(
    formatEventTime("2026-09-24T12:34:57.123Z"),
  );
});
