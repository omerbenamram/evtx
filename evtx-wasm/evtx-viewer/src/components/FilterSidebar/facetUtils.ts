import { levelName, type TableColumn, type FilterOptions } from "../../lib/types";
import {
  formatEventTime,
  formatTimeBucket,
  parseTimeRangeKey,
  timeRangeKey,
  type TimeZone,
} from "../../lib/timeZone";
import type { FacetConfig } from "./FacetSection";
import { parseSearchQuery, withoutField, withoutTerm, withTerm } from "../../lib/searchQuery";

/**
 * Built-in facets (level, time, provider, channel, eventId) followed by every
 * other active column, so users can filter on arbitrary extracted fields.
 */
export function buildFacetConfigs(columns: TableColumn[]): FacetConfig[] {
  const builtins: FacetConfig[] = [
    { id: "level", label: "Level", displayValue: levelName },
    {
      id: "time",
      label: "Date / Time",
      // Buckets are ranges; a cell's "Filter to this value" stays an exact timestamp.
      displayValue: (value, zone) =>
        parseTimeRangeKey(value) ? formatTimeBucket(value, zone) : formatEventTime(value, zone),
      chronological: true,
    },
    { id: "provider", label: "Provider" },
    { id: "channel", label: "Channel" },
    { id: "eventId", label: "Event ID" },
  ];
  const dynamic = columns
    .filter((column) => !builtins.some((facet) => facet.id === column.id))
    .map((column) => ({ id: column.id, label: column.header }));
  return [...builtins, ...dynamic];
}

/** `zone` defaults to View > Time zone. */
export function formatFacetValue(facet: FacetConfig, value: string, zone?: TimeZone): string {
  return value === "" ? "(Not set)" : (facet.displayValue?.(value, zone) ?? value);
}

const parsed = (filters: FilterOptions) => {
  try {
    return parseSearchQuery(filters.searchQuery ?? "");
  } catch {
    return {};
  }
};

/** Included values, which facet checkboxes show; excluded ones drop out of the counts. */
export function facetValues(filters: FilterOptions, id: string): string[] {
  const values = parsed(filters).include?.[id] ?? [];
  return id === "time" && filters.timeRange ? [...values, timeRangeKey(filters.timeRange)] : values;
}

export const excludedValues = (filters: FilterOptions, id: string): string[] =>
  parsed(filters).exclude?.[id] ?? [];

export const isFiltered = (filters: FilterOptions, id: string): boolean =>
  Boolean(facetValues(filters, id).length || excludedValues(filters, id).length);

export function clearFacet(filters: FilterOptions, id: string): FilterOptions {
  const searchQuery = withoutField(filters.searchQuery ?? "", id);
  return { ...filters, searchQuery, timeRange: id === "time" ? undefined : filters.timeRange };
}

/**
 * Like classList.toggle: `force` adds (true) or removes (false) instead of flipping. Edits the
 * query text; adding a value replaces its opposite, so a column never both keeps and drops it.
 */
export function toggleFacet(
  filters: FilterOptions,
  id: string,
  value: string,
  map: "include" | "exclude" = "include",
  force?: boolean,
): FilterOptions {
  // A time bucket sets the one time range, as the timeline does.
  const range = id === "time" && map === "include" ? parseTimeRangeKey(value) : undefined;
  if (range) {
    const on = facetValues(filters, id).includes(value);
    return on === (force ?? !on) ? filters : { ...filters, timeRange: on ? undefined : range };
  }
  const exclude = map === "exclude";
  const selected = (exclude ? excludedValues : facetValues)(filters, id).includes(value);
  if (selected === (force ?? !selected)) return filters;
  const query = filters.searchQuery ?? "";
  const searchQuery = selected
    ? withoutTerm(query, id, value, exclude)
    : withTerm(query, id, value, exclude);
  return { ...filters, searchQuery };
}
