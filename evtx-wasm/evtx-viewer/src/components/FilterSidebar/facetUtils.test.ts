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

it("edits the query text for one column without dropping other filters", () => {
  const initial = { searchQuery: "computer:HOST event_id:4624" };
  const added = toggleFacet(initial, "level", "2");
  expect(added).toEqual({ searchQuery: "computer:HOST event_id:4624 level:2" });
  expect(facetValues(added, "level")).toEqual(["2"]);
  expect(toggleFacet(added, "level", "2")).toEqual({
    searchQuery: "computer:HOST event_id:4624",
  });
  expect(clearFacet(added, "eventId")).toEqual({ searchQuery: "computer:HOST level:2" });
});

it("forces includes and excludes idempotently and keeps missing values distinct from zero", () => {
  const excluded = toggleFacet({}, "provider", "Security", "exclude", true);
  expect(excluded).toEqual({ searchQuery: "-provider:Security" });
  expect(toggleFacet(excluded, "provider", "Security", "exclude", true)).toBe(excluded);
  expect(toggleFacet(excluded, "provider", "Other", "exclude", false)).toBe(excluded);
  expect(facetValues(excluded, "provider")).toEqual([]);
  expect(isFiltered(excluded, "provider")).toBe(true);
  // Including a value replaces its exclusion; clearing drops both.
  const included = toggleFacet(excluded, "provider", "Security", "include", true);
  expect(included).toEqual({ searchQuery: "provider:Security" });
  expect(isFiltered(clearFacet(excluded, "provider"), "provider")).toBe(false);

  const selected = toggleFacet({ searchQuery: "level:0" }, "level", "");
  expect(selected).toEqual({ searchQuery: 'level:0 level:""' });
  expect(facetValues(selected, "level")).toEqual(["0", ""]);
  expect(toggleFacet(selected, "level", "")).toEqual({ searchQuery: "level:0" });
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

it("sets one time range from a bucket and shows it checked", () => {
  const bucket = "1475712000000/1475798400000";
  const selected = toggleFacet(
    { searchQuery: 'time:"2016-10-06T00:00:00.000000Z"' },
    "time",
    bucket,
  );
  expect(selected.timeRange).toEqual({
    start: new Date(1475712000000),
    end: new Date(1475798400000),
  });
  expect(facetValues(selected, "time")).toEqual(["2016-10-06T00:00:00.000000Z", bucket]);
  expect(toggleFacet(selected, "time", bucket).timeRange).toBeUndefined();
  expect(isFiltered(clearFacet(selected, "time"), "time")).toBe(false);
});
