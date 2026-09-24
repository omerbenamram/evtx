import { levelName, type TableColumn, type FilterOptions } from "../../lib/types";
import { formatEventTime, type TimeZone } from "../../lib/timeZone";
import type { FacetConfig } from "./FacetSection";

/**
 * Built-in facets (level, time, provider, channel, eventId) followed by every
 * other active column, so users can filter on arbitrary extracted fields.
 */
export function buildFacetConfigs(columns: TableColumn[]): FacetConfig[] {
  const builtins: FacetConfig[] = [
    { id: "level", label: "Level", displayValue: levelName },
    { id: "time", label: "Date / Time", displayValue: formatEventTime },
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

/** Included values, which facet checkboxes show; excluded ones drop out of the counts. */
export const facetValues = (filters: FilterOptions, id: string): string[] =>
  filters.include?.[id] ?? [];

export const isFiltered = (filters: FilterOptions, id: string): boolean =>
  Boolean(filters.include?.[id]?.length || filters.exclude?.[id]?.length);

export function clearFacet(filters: FilterOptions, id: string): FilterOptions {
  const include = { ...filters.include };
  const exclude = { ...filters.exclude };
  delete include[id];
  delete exclude[id];
  return { ...filters, include, exclude };
}

/**
 * Like classList.toggle: `force` adds (true) or removes (false) instead of flipping.
 * Adding a value to one map takes it out of the other, so a column never both keeps and drops it.
 */
export function toggleFacet(
  filters: FilterOptions,
  id: string,
  value: string,
  map: "include" | "exclude" = "include",
  force?: boolean,
): FilterOptions {
  const current = filters[map]?.[id] ?? [];
  const selected = current.includes(value);
  if (selected === (force ?? !selected)) return filters;
  const other = map === "include" ? "exclude" : "include";
  const next = {
    ...filters,
    [map]: {
      ...filters[map],
      [id]: selected ? current.filter((item) => item !== value) : [...current, value],
    },
  };
  const opposite = filters[other]?.[id];
  if (!selected && opposite?.includes(value))
    next[other] = { ...filters[other], [id]: opposite.filter((item) => item !== value) };
  return next;
}
