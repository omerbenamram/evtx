import React, { useMemo, useCallback, useState } from "react";
import styled, { css } from "styled-components";
import type { EvtxRecord, FilterOptions } from "../lib/types";
import {
  ChevronRight20Regular,
  ChevronDown20Regular,
  Search20Regular,
} from "@fluentui/react-icons";

interface FilterSidebarProps {
  records: EvtxRecord[];
  filters: FilterOptions;
  onChange: (filters: FilterOptions) => void;
}

const SidebarContainer = styled.div`
  display: flex;
  flex-direction: column;
  height: 100%;
  background: ${({ theme }) => theme.colors.background.secondary};
  padding-left: 3px; /* Account for the resize divider */
`;

const Header = styled.div`
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  font-weight: 600;
  background: ${({ theme }) => theme.colors.background.tertiary};
`;

const Section = styled.div``;

const SectionHeader = styled.button<{ $isOpen: boolean }>`
  display: flex;
  align-items: center;
  width: 100%;
  background: ${({ theme }) => theme.colors.background.secondary};
  border: none;
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  font-size: ${({ theme }) => theme.fontSize.body};
  cursor: pointer;
  color: ${({ theme }) => theme.colors.text.primary};
  user-select: none;
  transition: background-color ${({ theme }) => theme.transitions.fast};

  &:hover {
    background-color: ${({ theme }) => theme.colors.background.hover};
  }

  ${({ $isOpen, theme }) =>
    $isOpen &&
    css`
      background-color: ${theme.colors.background.hover};
    `}
`;

const SectionIcon = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  margin-right: ${({ theme }) => theme.spacing.sm};
`;

const OptionsContainer = styled.div`
  max-height: 240px;
  overflow: auto;
  padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.md}
    ${({ theme }) => theme.spacing.md};

  /* Bottom border when content is scrollable */
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
`;

const OptionRow = styled.label<{ $selected?: boolean }>`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: ${({ theme }) => theme.spacing.sm};
  padding: 4px 0;
  font-size: ${({ theme }) => theme.fontSize.caption};
  cursor: pointer;

  ${({ $selected, theme }) =>
    $selected &&
    css`
      color: ${theme.colors.accent.primary};
      font-weight: 600;
    `}
`;

const Counts = styled.span`
  color: ${({ theme }) => theme.colors.text.tertiary};
`;

const SearchContainer = styled.div`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.xs};
  background: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.light};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  padding: 4px 8px;
  margin: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  transition: border-color ${({ theme }) => theme.transitions.fast};

  &:focus-within {
    border-color: ${({ theme }) => theme.colors.accent.primary};
  }
`;

const SearchInput = styled.input`
  flex: 1;
  border: none;
  background: transparent;
  outline: none;
  font-size: ${({ theme }) => theme.fontSize.caption};
  color: ${({ theme }) => theme.colors.text.primary};

  &::placeholder {
    color: ${({ theme }) => theme.colors.text.tertiary};
  }
`;

const Checkbox = styled.input.attrs({ type: "checkbox" })`
  width: 14px;
  height: 14px;
  margin: 0;
  cursor: pointer;
  accent-color: ${({ theme }) => theme.colors.accent.primary};
`;

const OptionLabel = styled.span`
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

// Helper util
function increment(map: Map<string | number, number>, key: string | number) {
  map.set(key, (map.get(key) || 0) + 1);
}

export const FilterSidebar: React.FC<FilterSidebarProps> = ({
  records,
  filters,
  onChange,
}) => {
  // Compute facet counts
  const facetCounts = useMemo(() => {
    // Helper to test record against current filters, optionally ignoring one facet.
    const recordMatchesFilters = (
      rec: EvtxRecord,
      ignoreFacet?: keyof FilterOptions
    ) => {
      const sys = rec.Event.System ?? {};

      const term = (filters.searchTerm ?? "").toLowerCase();

      if (ignoreFacet !== "level" && filters.level && filters.level.length) {
        if (!filters.level.includes(sys.Level ?? 4)) return false;
      }

      if (
        ignoreFacet !== "provider" &&
        filters.provider &&
        filters.provider.length
      ) {
        if (!filters.provider.includes(sys.Provider?.Name ?? "")) return false;
      }

      if (
        ignoreFacet !== "channel" &&
        filters.channel &&
        filters.channel.length
      ) {
        if (!filters.channel.includes(sys.Channel ?? "")) return false;
      }

      if (
        ignoreFacet !== "eventId" &&
        filters.eventId &&
        filters.eventId.length
      ) {
        const idNum =
          typeof sys.EventID === "string"
            ? parseInt(sys.EventID, 10)
            : sys.EventID;
        if (!filters.eventId.includes(Number(idNum))) return false;
      }

      if (term !== "") {
        const searchStr = `${sys.Provider?.Name ?? ""} ${sys.Computer ?? ""} ${
          sys.EventID ?? ""
        }`.toLowerCase();
        if (!searchStr.includes(term)) return false;
      }

      return true;
    };

    const level = new Map<number, number>();
    const provider = new Map<string, number>();
    const channel = new Map<string, number>();
    const eventId = new Map<number, number>();

    records.forEach((rec) => {
      const sys = rec.Event.System ?? {};

      // For each facet we compute counts using records that satisfy all other facets.

      if (recordMatchesFilters(rec, "level")) {
        increment(level, sys.Level ?? 4);
      }

      if (recordMatchesFilters(rec, "provider")) {
        if (sys.Provider?.Name) increment(provider, sys.Provider.Name);
      }

      if (recordMatchesFilters(rec, "channel")) {
        if (sys.Channel) increment(channel, sys.Channel);
      }

      if (recordMatchesFilters(rec, "eventId")) {
        if (sys.EventID !== undefined && sys.EventID !== null) {
          const idNum =
            typeof sys.EventID === "string"
              ? parseInt(sys.EventID, 10)
              : sys.EventID;
          if (!Number.isNaN(idNum)) increment(eventId, idNum);
        }
      }
    });

    return {
      level,
      provider,
      channel,
      eventId,
    } as const;
  }, [records, filters]);

  // Collapsed state per section
  const [openSections, setOpenSections] = useState<Record<string, boolean>>({
    level: true,
    provider: true,
    channel: true,
    eventId: false,
  });

  const toggleSection = useCallback((key: string) => {
    setOpenSections((prev) => ({ ...prev, [key]: !prev[key] }));
  }, []);

  // Search state per section (for provider/channel/eventId maybe heavy)
  const [searchTerms, setSearchTerms] = useState<Record<string, string>>({});

  const handleSearchChange = useCallback((section: string, term: string) => {
    setSearchTerms((prev) => ({ ...prev, [section]: term.toLowerCase() }));
  }, []);

  // Handlers for toggling option selections
  const toggleFilterValue = useCallback(
    (facet: keyof FilterOptions, value: string | number) => {
      const current = (filters[facet] as (string | number)[] | undefined) ?? [];
      const exists = current.includes(value);
      const newVals = exists
        ? current.filter((v) => v !== value)
        : [...current, value];
      onChange({ ...filters, [facet]: newVals });
    },
    [filters, onChange]
  );

  // Render helper for option list
  const renderOptions = (
    map: Map<string | number, number>,
    facetKey: keyof FilterOptions,
    displayValue?: (v: string | number) => string
  ) => {
    // sort by count desc
    const term = (searchTerms[facetKey as string] ?? "").toLowerCase();
    const entries = Array.from(map.entries())
      .filter(([key]) =>
        term === "" ? true : String(key).toLowerCase().includes(term)
      )
      .sort((a, b) => b[1] - a[1])
      .slice(0, 200); // cap for perf

    return entries.map(([val, count]) => {
      const selected = Boolean(
        (filters[facetKey] as (string | number)[] | undefined)?.includes(val)
      );
      return (
        <OptionRow key={String(val)} $selected={selected}>
          <Checkbox
            checked={selected}
            onChange={() => toggleFilterValue(facetKey, val)}
          />
          <OptionLabel>
            {displayValue ? displayValue(val) : String(val)}
          </OptionLabel>
          <Counts>{count}</Counts>
        </OptionRow>
      );
    });
  };

  const LEVEL_NAME_MAP: Record<number, string> = {
    0: "LogAlways",
    1: "Critical",
    2: "Error",
    3: "Warning",
    4: "Information",
    5: "Verbose",
  };

  return (
    <SidebarContainer>
      <Header>Filters</Header>

      {/* Search term global */}
      <SearchContainer>
        <Search20Regular />
        <SearchInput
          placeholder="Search all..."
          value={filters.searchTerm ?? ""}
          onChange={(e) => onChange({ ...filters, searchTerm: e.target.value })}
        />
      </SearchContainer>

      {/* Level Section */}
      <Section>
        <SectionHeader
          $isOpen={openSections.level}
          onClick={() => toggleSection("level")}
        >
          <SectionIcon>
            {openSections.level ? (
              <ChevronDown20Regular />
            ) : (
              <ChevronRight20Regular />
            )}
          </SectionIcon>
          Level
        </SectionHeader>
        {openSections.level && (
          <OptionsContainer>
            {renderOptions(
              facetCounts.level,
              "level",
              (v) => LEVEL_NAME_MAP[v as number] || String(v)
            )}
          </OptionsContainer>
        )}
      </Section>

      {/* Provider Section */}
      <Section>
        <SectionHeader
          $isOpen={openSections.provider}
          onClick={() => toggleSection("provider")}
        >
          <SectionIcon>
            {openSections.provider ? (
              <ChevronDown20Regular />
            ) : (
              <ChevronRight20Regular />
            )}
          </SectionIcon>
          Provider
        </SectionHeader>
        {openSections.provider && (
          <>
            <SearchContainer>
              <Search20Regular />
              <SearchInput
                placeholder="Search provider..."
                value={searchTerms.provider ?? ""}
                onChange={(e) => handleSearchChange("provider", e.target.value)}
              />
            </SearchContainer>
            <OptionsContainer>
              {renderOptions(facetCounts.provider, "provider")}
            </OptionsContainer>
          </>
        )}
      </Section>

      {/* Channel Section */}
      <Section>
        <SectionHeader
          $isOpen={openSections.channel}
          onClick={() => toggleSection("channel")}
        >
          <SectionIcon>
            {openSections.channel ? (
              <ChevronDown20Regular />
            ) : (
              <ChevronRight20Regular />
            )}
          </SectionIcon>
          Channel
        </SectionHeader>
        {openSections.channel && (
          <>
            <SearchContainer>
              <Search20Regular />
              <SearchInput
                placeholder="Search channel..."
                value={searchTerms.channel ?? ""}
                onChange={(e) => handleSearchChange("channel", e.target.value)}
              />
            </SearchContainer>
            <OptionsContainer>
              {renderOptions(facetCounts.channel, "channel")}
            </OptionsContainer>
          </>
        )}
      </Section>

      {/* EventID Section */}
      <Section>
        <SectionHeader
          $isOpen={openSections.eventId}
          onClick={() => toggleSection("eventId")}
        >
          <SectionIcon>
            {openSections.eventId ? (
              <ChevronDown20Regular />
            ) : (
              <ChevronRight20Regular />
            )}
          </SectionIcon>
          Event ID
        </SectionHeader>
        {openSections.eventId && (
          <OptionsContainer>
            {renderOptions(facetCounts.eventId, "eventId")}
          </OptionsContainer>
        )}
      </Section>
    </SidebarContainer>
  );
};
