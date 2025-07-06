import React, { useMemo, useCallback, useState } from "react";
// styled components are now centralised in styles.ts (avoids duplication)
import {
  SidebarContainer,
  ActiveFiltersBar,
  FilterChip,
  ChipRemoveBtn,
} from "./styles";
import type { FilterOptions } from "../../lib/types";
import { Search20Regular, Dismiss16Regular } from "@fluentui/react-icons";
import { Button } from "../Windows";
import { SidebarHeader } from "../Windows";
import { logger } from "../../lib/logger";
import FacetSection from "./FacetSection";
import type { FacetConfig } from "./FacetSection";
import { useFacetCounts } from "./useFacetCounts";
import { buildFacetConfigs } from "./facetUtils";
import { useActiveFilterChips } from "./useActiveFilterChips";
import { useColumns } from "../../hooks/useColumns";
import { useFilters } from "../../hooks/useFilters";
import { useIngestState } from "../../state/store";
import { SearchContainer, SearchInput } from "../Windows";

// (increment helper removed – facet counts are now sourced exclusively from DuckDB)

export const FilterSidebar: React.FC = () => {
  const { filters, setFilters } = useFilters();
  const { columns } = useColumns();
  const { progress: ingestProgress } = useIngestState();

  const onChange = setFilters;

  const filtersDisabled = ingestProgress < 1;

  /* Debug: log whenever filters prop changes */
  React.useEffect(() => {
    logger.debug("FilterSidebar filters prop changed", { filters });
  }, [filters]);

  // ---------------- Unified facet counts via hook ----------------
  const dynCounts = useFacetCounts();
  // ---------------- All facet counts storage ----------------

  // ---------------------------------------------------------------------------
  // Active filter chips – generic builder using facetConfigs
  // ---------------------------------------------------------------------------
  /* activeChips is defined later, after facetConfigs & toggleFacetValue */

  // ---------------------------------------------------------------------------
  // Counts resolver – single source (dynCounts)
  // ---------------------------------------------------------------------------
  const getCountsForFacet = useCallback(
    (id: string): Map<string | number, number> => dynCounts[id] ?? new Map(),
    [dynCounts]
  );

  // ---------------------------------------------------------------------------
  // Per-section UI state (collapse + search boxes)
  // ---------------------------------------------------------------------------

  const [openSections, setOpenSections] = useState<Record<string, boolean>>({
    level: true,
    provider: true,
    channel: true,
    eventId: false,
  });

  const toggleSection = useCallback((key: string) => {
    setOpenSections((prev) => ({ ...prev, [key]: !prev[key] }));
  }, []);

  const [searchTerms, setSearchTerms] = useState<Record<string, string>>({});
  const handleSearchChange = useCallback((section: string, term: string) => {
    setSearchTerms((prev) => ({ ...prev, [section]: term.toLowerCase() }));
  }, []);

  // ---------------------------------------------------------------------------
  // Facet configuration (built-ins + dynamic columns)
  // ---------------------------------------------------------------------------

  const facetConfigs: FacetConfig[] = React.useMemo(
    () => buildFacetConfigs(columns),
    [columns]
  );

  // ---------------------------------------------------------------------------
  // Generic toggle handler – supports both built-in filters and columnEquals.
  // ---------------------------------------------------------------------------
  const toggleFacetValue = useCallback(
    (facet: FacetConfig, value: string | number) => {
      logger.debug("toggleFacetValue", { facet: facet.id, value });
      if (facet.filterKey) {
        // Built-in simple array filter
        const key = facet.filterKey;
        const current = (filters[key] as (string | number)[] | undefined) ?? [];
        const exists = current.includes(value);
        const nextVals = exists
          ? current.filter((v) => v !== value)
          : [...current, value];
        const next = { ...filters, [key]: nextVals } as FilterOptions;
        onChange(next);
      } else {
        // Column equality filter
        const colId = facet.id;
        const current = filters.columnEquals?.[colId] ?? [];
        const exists = current.includes(value as string);
        const nextVals = exists
          ? current.filter((v) => v !== value)
          : [...current, value as string];
        const nextMap = { ...(filters.columnEquals ?? {}) } as Record<
          string,
          string[]
        >;
        if (nextVals.length) nextMap[colId] = nextVals;
        else delete nextMap[colId];
        onChange({ ...filters, columnEquals: nextMap });
      }
    },
    [filters, onChange]
  );

  // ---------------------------------------------------------------------------
  // Active filter chips – generic builder using facetConfigs
  // ---------------------------------------------------------------------------
  const activeChips = useActiveFilterChips(
    facetConfigs,
    onChange,
    toggleFacetValue
  );

  // ---------------------------------------------------------------------------
  // Helpers for FacetSection
  // ---------------------------------------------------------------------------

  const selectedCheckerFactory =
    (facet: FacetConfig) => (val: string | number) => {
      if (facet.filterKey) {
        const current =
          (filters[facet.filterKey] as (string | number)[] | undefined) ?? [];
        return current.includes(val);
      }
      return (filters.columnEquals?.[facet.id] ?? []).includes(val as string);
    };

  // ---------------------------------------------------------------------------
  // Determine if any filters are currently active (unchanged)
  // ---------------------------------------------------------------------------
  const hasActiveFilters = useMemo(() => {
    return (
      (filters.searchTerm && filters.searchTerm.trim() !== "") ||
      (filters.level && filters.level.length > 0) ||
      (filters.provider && filters.provider.length > 0) ||
      (filters.channel && filters.channel.length > 0) ||
      (filters.eventId && filters.eventId.length > 0) ||
      (filters.eventData && Object.keys(filters.eventData).length > 0) ||
      (filters.eventDataExclude &&
        Object.keys(filters.eventDataExclude).length > 0) ||
      (filters.columnEquals && Object.keys(filters.columnEquals).length > 0)
    );
  }, [filters]);

  return (
    <SidebarContainer>
      <SidebarHeader>
        <span>Filters</span>
        {hasActiveFilters && !filtersDisabled && (
          <Button variant="subtle" size="small" onClick={() => onChange({})}>
            Clear
          </Button>
        )}
      </SidebarHeader>

      {filtersDisabled ? (
        <div
          style={{
            padding: "16px",
            fontSize: "0.875rem",
            color: "var(--text-tertiary, #666)",
          }}
        >
          Filtering unavailable – database not initialised.
        </div>
      ) : (
        <>
          {activeChips.length > 0 && (
            <ActiveFiltersBar>
              {activeChips.map((chip) => (
                <FilterChip key={chip.key}>
                  {chip.label}
                  <ChipRemoveBtn onClick={chip.remove} title="Remove filter">
                    <Dismiss16Regular />
                  </ChipRemoveBtn>
                </FilterChip>
              ))}
            </ActiveFiltersBar>
          )}
          {/* Search term global */}
          <SearchContainer>
            <Search20Regular />
            <SearchInput
              placeholder="Search all..."
              value={filters.searchTerm ?? ""}
              onChange={(e) =>
                onChange({ ...filters, searchTerm: e.target.value })
              }
            />
          </SearchContainer>

          {/* Unified facet sections */}
          {facetConfigs.map((facet) => (
            <FacetSection
              key={facet.id}
              facet={facet}
              counts={getCountsForFacet(facet.id)}
              isOpen={openSections[facet.id] ?? true}
              searchTerm={searchTerms[facet.id] ?? ""}
              toggleOpen={toggleSection}
              onSearchTermChange={handleSearchChange}
              toggleFacetValue={toggleFacetValue}
              selectedChecker={selectedCheckerFactory(facet)}
            />
          ))}
        </>
      )}
    </SidebarContainer>
  );
};
