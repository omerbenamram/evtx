import { useMemo } from "react";
import type { FacetConfig } from "./FacetSection";
import type { FilterOptions } from "../../lib/types";
import { useFiltersState } from "../../state/store";

export interface ActiveChip {
  key: string;
  label: string;
  remove: () => void;
}

/**
 * Derive the list of active filter "chips" for display in the sidebar.
 * All remove callbacks are fully memoised so the consumer can pass them
 * straight to the chip UI component without additional wrappers.
 */
export function useActiveFilterChips(
  facetConfigs: FacetConfig[],
  onChange: (next: FilterOptions) => void,
  toggleFacetValue: (facet: FacetConfig, value: string | number) => void
): ActiveChip[] {
  const filters = useFiltersState();
  return useMemo(() => {
    const chips: ActiveChip[] = [];

    // Global search term
    if (filters.searchTerm && filters.searchTerm.trim() !== "") {
      chips.push({
        key: "search",
        label: `Search: "${filters.searchTerm.trim()}"`,
        remove: () => onChange({ ...filters, searchTerm: "" }),
      });
    }

    // Facet-based chips (built-ins + dynamic columns)
    facetConfigs.forEach((facet) => {
      let values: (string | number)[] = [];
      if (facet.filterKey) {
        values =
          (filters[facet.filterKey as keyof FilterOptions] as
            | (string | number)[]
            | undefined) ?? [];
      } else {
        values = filters.columnEquals?.[facet.id] ?? [];
      }

      values.forEach((v) => {
        const display = facet.displayValue ? facet.displayValue(v) : String(v);
        const label = `${facet.label}: ${display}`;
        chips.push({
          key: `${facet.id}-${v}`,
          label,
          remove: () => toggleFacetValue(facet, v),
        });
      });
    });

    // EventData include chips
    (filters.eventData ? Object.entries(filters.eventData) : []).forEach(
      ([field, vals]) => {
        vals.forEach((v) => {
          chips.push({
            key: `ed-${field}-${v}`,
            label: `${field}: ${v}`,
            remove: () => {
              const currentVals = filters.eventData![field] ?? [];
              const newVals = currentVals.filter((x) => x !== v);
              const newEventData = { ...filters.eventData } as Record<
                string,
                string[]
              >;
              if (newVals.length) newEventData[field] = newVals;
              else delete newEventData[field];
              onChange({ ...filters, eventData: newEventData });
            },
          });
        });
      }
    );

    // EventData exclude chips
    (filters.eventDataExclude
      ? Object.entries(filters.eventDataExclude)
      : []
    ).forEach(([field, vals]) => {
      vals.forEach((v) => {
        chips.push({
          key: `edex-${field}-${v}`,
          label: `¬${field}: ${v}`,
          remove: () => {
            const currentVals = filters.eventDataExclude![field] ?? [];
            const newVals = currentVals.filter((x) => x !== v);
            const newEventDataEx = { ...filters.eventDataExclude } as Record<
              string,
              string[]
            >;
            if (newVals.length) newEventDataEx[field] = newVals;
            else delete newEventDataEx[field];
            onChange({ ...filters, eventDataExclude: newEventDataEx });
          },
        });
      });
    });

    return chips;
  }, [filters, facetConfigs, onChange, toggleFacetValue]);
}
