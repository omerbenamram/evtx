import React from "react";
import {
  SectionHeader,
  SectionIcon,
  OptionsContainer,
  Counts,
  Checkbox,
  OptionLabel,
} from "./styles";
import {
  ChevronRight20Regular,
  ChevronDown20Regular,
  Search20Regular,
} from "@fluentui/react-icons";
import { SearchContainer, SearchInput, SelectableRow } from "../Windows";
import { formatFacetValue } from "./facetUtils";

export interface FacetConfig {
  id: string;
  label: string;
  searchable?: boolean;
  displayValue?: (value: string) => string;
}

interface FacetSectionProps {
  facet: FacetConfig;
  counts: Map<string, number>;
  isOpen: boolean;
  searchTerm: string;
  toggleOpen: (key: string) => void;
  onSearchTermChange: (key: string, term: string) => void;
  toggleFacetValue: (id: string, value: string) => void;
  selected: string[];
}

const FacetSection: React.FC<FacetSectionProps> = ({
  facet,
  counts,
  isOpen,
  searchTerm,
  toggleOpen,
  onSearchTermChange,
  toggleFacetValue,
  selected,
}) => {
  const entries = React.useMemo(() => {
    const term = searchTerm.toLowerCase();
    return Array.from(counts.entries())
      .filter(([key]) => formatFacetValue(facet, key).toLowerCase().includes(term))
      .toSorted((a, b) => b[1] - a[1]);
  }, [counts, searchTerm, facet]);

  return (
    <div>
      <SectionHeader
        type="button"
        aria-expanded={isOpen}
        $isOpen={isOpen}
        onClick={() => toggleOpen(facet.id)}
      >
        <SectionIcon>{isOpen ? <ChevronDown20Regular /> : <ChevronRight20Regular />}</SectionIcon>
        {facet.label}
      </SectionHeader>
      {isOpen && (
        <>
          {facet.searchable && (
            <SearchContainer $compact style={{ margin: "4px 12px" }}>
              <Search20Regular />
              <SearchInput
                aria-label={`Search ${facet.label.toLowerCase()}`}
                placeholder={`Search ${facet.label.toLowerCase()}...`}
                value={searchTerm}
                onChange={(e) => onSearchTermChange(facet.id, e.target.value)}
              />
            </SearchContainer>
          )}
          <OptionsContainer>
            {entries.map(([val, count]) => {
              const checked = selected.includes(val);
              return (
                <SelectableRow key={val} $selected={checked}>
                  <Checkbox checked={checked} onChange={() => toggleFacetValue(facet.id, val)} />
                  <OptionLabel title={val || "(Not set)"}>
                    {formatFacetValue(facet, val)}
                  </OptionLabel>
                  <Counts>{count}</Counts>
                </SelectableRow>
              );
            })}
          </OptionsContainer>
        </>
      )}
    </div>
  );
};

export default FacetSection;
