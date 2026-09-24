import React from "react";
import {
  Section,
  SectionHeader,
  SectionToggle,
  SelectedCount,
  FacetSearch,
  OptionsContainer,
  FacetRow,
  Counts,
  Checkbox,
  OptionLabel,
} from "./styles";
import {
  ChevronRight16Regular,
  ChevronDown16Regular,
  Search16Regular,
} from "@fluentui/react-icons";
import { Button, SearchContainer, SearchInput, Tooltip } from "../Windows";
import { formatFacetValue } from "./facetUtils";
import { useTimeZone, type TimeZone } from "../../lib/timeZone";

export interface FacetConfig {
  id: string;
  label: string;
  displayValue?: (value: string, zone?: TimeZone) => string;
  /** Keep the counts' order (oldest first) instead of ranking by count. */
  chronological?: boolean;
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
  /** Included plus excluded values; 0 hides Clear. */
  filteredCount: number;
  onClear: (id: string) => void;
}

// ponytail: tooltip only for values likely to truncate; measure scrollWidth if short ones clip.
const LONG_VALUE = 24;

const FacetSection: React.FC<FacetSectionProps> = ({
  facet,
  counts,
  isOpen,
  searchTerm,
  toggleOpen,
  onSearchTermChange,
  toggleFacetValue,
  selected,
  filteredCount,
  onClear,
}) => {
  const zone = useTimeZone();
  const entries = React.useMemo(() => {
    const term = searchTerm.toLowerCase();
    const shown = Array.from(counts.entries(), ([value, count]) => ({
      value,
      count,
      label: formatFacetValue(facet, value, zone),
    })).filter(({ label }) => label.toLowerCase().includes(term));
    return facet.chronological ? shown : shown.toSorted((a, b) => b.count - a.count);
  }, [counts, searchTerm, facet, zone]);
  let total = 0;
  for (const count of counts.values()) total += count;

  return (
    <Section>
      <SectionHeader>
        <SectionToggle type="button" aria-expanded={isOpen} onClick={() => toggleOpen(facet.id)}>
          {isOpen ? <ChevronDown16Regular /> : <ChevronRight16Regular />}
          <span>{facet.label}</span>
          {filteredCount > 0 && (
            <SelectedCount aria-label={`${filteredCount} filtered`}>{filteredCount}</SelectedCount>
          )}
        </SectionToggle>
        {filteredCount > 0 && (
          <Button
            variant="subtle"
            aria-label={`Clear ${facet.label} filter`}
            onClick={() => onClear(facet.id)}
          >
            Clear
          </Button>
        )}
      </SectionHeader>
      {isOpen && (
        <>
          {counts.size > 10 && (
            <FacetSearch>
              <SearchContainer>
                <Search16Regular />
                <SearchInput
                  aria-label={`Search ${facet.label}`}
                  placeholder="Search"
                  value={searchTerm}
                  onChange={(e) => onSearchTermChange(facet.id, e.target.value)}
                />
              </SearchContainer>
            </FacetSearch>
          )}
          <OptionsContainer>
            {entries.map(({ value, count, label }) => {
              const text = <OptionLabel>{label}</OptionLabel>;
              return (
                // SAFETY: `--share` is a CSS custom property, which CSSProperties does not list.
                <FacetRow
                  key={value}
                  style={{ "--share": `${(count / total) * 100}%` } as React.CSSProperties}
                >
                  <Checkbox
                    checked={selected.includes(value)}
                    onChange={() => toggleFacetValue(facet.id, value)}
                  />
                  {label.length > LONG_VALUE ? <Tooltip label={label}>{text}</Tooltip> : text}
                  <Counts>{count.toLocaleString()}</Counts>
                </FacetRow>
              );
            })}
          </OptionsContainer>
        </>
      )}
    </Section>
  );
};

export default FacetSection;
