import React from "react";
import {
  Section,
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
import type { FilterOptions } from "../../lib/types";

// ---------------- Types ----------------

export interface FacetConfig {
  id: string;
  label: string;
  filterKey?: keyof FilterOptions;
  searchable?: boolean;
  displayValue?: (v: string | number) => string;
}

interface FacetSectionProps {
  facet: FacetConfig;
  counts: Map<string | number, number>;
  isOpen: boolean;
  searchTerm: string;
  toggleOpen: (key: string) => void;
  onSearchTermChange: (key: string, term: string) => void;
  toggleFacetValue: (facet: FacetConfig, value: string | number) => void;
  selectedChecker: (val: string | number) => boolean;
}

const FacetSection: React.FC<FacetSectionProps> = ({
  facet,
  counts,
  isOpen,
  searchTerm,
  toggleOpen,
  onSearchTermChange,
  toggleFacetValue,
  selectedChecker,
}) => {
  const entries = React.useMemo(() => {
    const term = searchTerm.toLowerCase();
    return Array.from(counts.entries())
      .filter(([k]) =>
        term === "" ? true : String(k).toLowerCase().includes(term)
      )
      .sort((a, b) => b[1] - a[1])
      .slice(0, 200);
  }, [counts, searchTerm]);

  return (
    <Section>
      <SectionHeader $isOpen={isOpen} onClick={() => toggleOpen(facet.id)}>
        <SectionIcon>
          {isOpen ? <ChevronDown20Regular /> : <ChevronRight20Regular />}
        </SectionIcon>
        {facet.label}
      </SectionHeader>
      {isOpen && (
        <>
          {facet.searchable && (
            <SearchContainer>
              <Search20Regular />
              <SearchInput
                placeholder={`Search ${facet.label.toLowerCase()}...`}
                value={searchTerm}
                onChange={(e) => onSearchTermChange(facet.id, e.target.value)}
              />
            </SearchContainer>
          )}
          <OptionsContainer>
            {entries.map(([val, count]) => {
              const selected = selectedChecker(val);
              return (
                <SelectableRow key={String(val)} $selected={selected}>
                  <Checkbox
                    checked={selected}
                    onChange={() => toggleFacetValue(facet, val)}
                  />
                  <OptionLabel>
                    {facet.displayValue ? facet.displayValue(val) : String(val)}
                  </OptionLabel>
                  <Counts>{count}</Counts>
                </SelectableRow>
              );
            })}
          </OptionsContainer>
        </>
      )}
    </Section>
  );
};

export default FacetSection;
