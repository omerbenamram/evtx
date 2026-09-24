import { formatFacetValue } from "./facetUtils";
import { useMemo } from "react";
import type { FacetConfig } from "./FacetSection";
import { eventDataField } from "../../lib/columnSql";
import { queryTerms, withoutTerm, type QueryTerm } from "../../lib/searchQuery";
import { formatEventTime, timeZoneLabel, useTimeZone } from "../../lib/timeZone";
import { useFilters } from "../../hooks/useFilters";

interface ActiveChip {
  key: string;
  label: string;
  remove: () => void;
}

const TEXT_FACET: FacetConfig = { id: "", label: "Text" };

/** One chip per query term (removing it edits the query text), plus the time range. */
export function useActiveFilterChips(facetConfigs: FacetConfig[]): ActiveChip[] {
  const { filters, updateFilters } = useFilters();
  const zone = useTimeZone();
  return useMemo(() => {
    const chips: ActiveChip[] = [];
    if (filters.timeRange) {
      const { start, end } = filters.timeRange;
      chips.push({
        key: "timeRange",
        label: `Time: ${formatEventTime(start, zone)} – ${formatEventTime(end, zone)} ${timeZoneLabel(zone)}`,
        remove: () => updateFilters((current) => ({ ...current, timeRange: undefined })),
      });
    }
    let terms: QueryTerm[];
    try {
      terms = queryTerms(filters.searchQuery ?? "");
    } catch {
      terms = [];
    }
    // Hidden columns may still have filters; keep those removable too.
    terms.forEach(({ id, values: [value], exclude }, index) => {
      const facet = !id
        ? TEXT_FACET
        : (facetConfigs.find((item) => item.id === id) ?? { id, label: eventDataField(id) ?? id });
      chips.push({
        key: `${index}-${id}-${value}`,
        label: `${facet.label}${exclude ? " is not" : ":"} ${formatFacetValue(facet, value, zone)}`,
        remove: () =>
          updateFilters((current) => ({
            ...current,
            searchQuery: withoutTerm(current.searchQuery ?? "", id, value, exclude),
          })),
      });
    });
    return chips;
  }, [filters, facetConfigs, updateFilters, zone]);
}
