import React, { useCallback, useState } from "react";
import { Notice, SidebarBody, SidebarContainer } from "./styles";
import { Button, SidebarHeader } from "../Windows";
import FacetSection from "./FacetSection";
import { useFacetCounts } from "./useFacetCounts";
import {
  buildFacetConfigs,
  clearFacet,
  excludedValues,
  facetValues,
  toggleFacet,
} from "./facetUtils";
import { useActiveFilterChips } from "./useActiveFilterChips";
import { useColumns } from "../../hooks/useColumns";
import { useFilters } from "../../hooks/useFilters";
import { useEvtxMetaState } from "../../state/store";

export const FilterSidebar: React.FC = () => {
  const { filters, updateFilters, clearFilters } = useFilters();
  const { columns } = useColumns();
  const { isLoading, ingestProgress } = useEvtxMetaState();

  // A cancelled or failed import stops loading below 100%; its events stay filterable.
  const filtersDisabled = isLoading && ingestProgress < 1;

  const dynCounts = useFacetCounts();

  const [openSections, setOpenSections] = useState<Record<string, boolean>>({ eventId: false });

  const toggleSection = useCallback((key: string) => {
    setOpenSections((prev) => ({ ...prev, [key]: !(prev[key] ?? true) }));
  }, []);

  const [searchTerms, setSearchTerms] = useState<Record<string, string>>({});
  const handleSearchChange = useCallback((section: string, term: string) => {
    setSearchTerms((prev) => ({ ...prev, [section]: term }));
  }, []);

  const facetConfigs = React.useMemo(() => buildFacetConfigs(columns), [columns]);
  const toggleFacetValue = useCallback(
    (id: string, value: string) => updateFilters((current) => toggleFacet(current, id, value)),
    [updateFilters],
  );
  const clearSection = useCallback(
    (id: string) => updateFilters((current) => clearFacet(current, id)),
    [updateFilters],
  );

  const hasActiveFilters = useActiveFilterChips(facetConfigs).length > 0;

  return (
    <SidebarContainer>
      <SidebarHeader>
        <span>Filters</span>
        {hasActiveFilters && !filtersDisabled && (
          <Button variant="subtle" onClick={clearFilters}>
            Clear all
          </Button>
        )}
      </SidebarHeader>

      {filtersDisabled ? (
        <Notice>Available after import.</Notice>
      ) : (
        <SidebarBody>
          {facetConfigs.map((facet) => (
            <FacetSection
              key={facet.id}
              facet={facet}
              counts={dynCounts[facet.id] ?? new Map()}
              isOpen={openSections[facet.id] ?? true}
              searchTerm={searchTerms[facet.id] ?? ""}
              toggleOpen={toggleSection}
              onSearchTermChange={handleSearchChange}
              toggleFacetValue={toggleFacetValue}
              selected={facetValues(filters, facet.id)}
              filteredCount={
                facetValues(filters, facet.id).length + excludedValues(filters, facet.id).length
              }
              onClear={clearSection}
            />
          ))}
        </SidebarBody>
      )}
    </SidebarContainer>
  );
};
