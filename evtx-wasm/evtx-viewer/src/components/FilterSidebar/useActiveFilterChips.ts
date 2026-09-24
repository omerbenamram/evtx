import { formatFacetValue, toggleFacet } from "./facetUtils";
import { useMemo } from "react";
import type { FacetConfig } from "./FacetSection";
import { eventDataField } from "../../lib/columnSql";
import { formatEventTime, timeZoneLabel, useTimeZone } from "../../lib/timeZone";
import { useFilters } from "../../hooks/useFilters";

interface ActiveChip {
  key: string;
  label: string;
  remove: () => void;
}

export function useActiveFilterChips(facetConfigs: FacetConfig[]): ActiveChip[] {
  const { filters, updateFilters } = useFilters();
  const zone = useTimeZone();
  return useMemo(() => {
    const chips: ActiveChip[] = [];

    const query = filters.searchQuery?.trim();
    if (query) {
      chips.push({
        key: "query",
        label: `Search: ${query}`,
        remove: () => updateFilters((current) => ({ ...current, searchQuery: "" })),
      });
    }
    if (filters.timeRange) {
      const { start, end } = filters.timeRange;
      chips.push({
        key: "timeRange",
        label: `Time: ${formatEventTime(start, zone)} – ${formatEventTime(end, zone)} ${timeZoneLabel(zone)}`,
        remove: () => updateFilters((current) => ({ ...current, timeRange: undefined })),
      });
    }
    // Hidden columns may still have filters; keep those removable too.
    for (const map of ["include", "exclude"] as const) {
      for (const [id, values] of Object.entries(filters[map] ?? {})) {
        const facet = facetConfigs.find((item) => item.id === id) ?? {
          id,
          label: eventDataField(id) ?? id,
        };
        for (const value of values) {
          chips.push({
            key: `${map}-${id}-${value}`,
            label: `${facet.label}${map === "exclude" ? " is not" : ":"} ${formatFacetValue(facet, value, zone)}`,
            remove: () => updateFilters((current) => toggleFacet(current, id, value, map, false)),
          });
        }
      }
    }

    return chips;
  }, [filters, facetConfigs, updateFilters, zone]);
}
