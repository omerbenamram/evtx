import React, { useMemo, useCallback, useState } from "react";
import styled, { css } from "styled-components";
import type { EvtxRecord, FilterOptions, BucketCounts } from "../lib/types";
import {
  ChevronRight20Regular,
  ChevronDown20Regular,
  Search20Regular,
  Dismiss16Regular,
} from "@fluentui/react-icons";
import { Button } from "./Windows";
import { logger } from "../lib/logger";

interface FilterSidebarProps {
  records: EvtxRecord[];
  filters: FilterOptions;
  bucketCounts?: BucketCounts | null;
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
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: ${({ theme }) => theme.spacing.sm};
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

const ActiveFiltersBar = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  background: ${({ theme }) => theme.colors.background.tertiary};
`;

const FilterChip = styled.span`
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 6px;
  background: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  font-size: ${({ theme }) => theme.fontSize.caption};
  color: ${({ theme }) => theme.colors.text.primary};
`;

const ChipRemoveBtn = styled.button`
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
  color: inherit;
  line-height: 1;
`;

// (increment helper removed – facet counts are now sourced exclusively from DuckDB)

export const FilterSidebar: React.FC<FilterSidebarProps> = (props) => {
  const { filters, bucketCounts, onChange } = props;

  const filtersDisabled = !bucketCounts;

  /* Debug: log whenever filters *prop* changes so we can correlate with
   * LogTableVirtual behaviour.
   */
  React.useEffect(() => {
    logger.debug("FilterSidebar filters prop changed", { filters });
  }, [filters]);

  const LEVEL_NAME_MAP: Record<number, string> = {
    0: "LogAlways",
    1: "Critical",
    2: "Error",
    3: "Warning",
    4: "Information",
    5: "Verbose",
  };

  // Build list of active filters for chip display
  const activeChips: { key: string; label: string; remove: () => void }[] = [];

  // Search term
  if (filters.searchTerm && filters.searchTerm.trim() !== "") {
    activeChips.push({
      key: `search`,
      label: `Search: "${filters.searchTerm.trim()}"`,
      remove: () => onChange({ ...filters, searchTerm: "" }),
    });
  }

  // Level
  if (filters.level && filters.level.length) {
    filters.level.forEach((lvl) => {
      const lbl = LEVEL_NAME_MAP[lvl] || String(lvl);
      activeChips.push({
        key: `level-${lvl}`,
        label: `Level: ${lbl}`,
        remove: () => {
          const rem = filters.level!.filter((l) => l !== lvl);
          onChange({ ...filters, level: rem });
        },
      });
    });
  }

  // Provider
  if (filters.provider && filters.provider.length) {
    filters.provider.forEach((p) => {
      activeChips.push({
        key: `prov-${p}`,
        label: `Provider: ${p}`,
        remove: () => {
          const rem = filters.provider!.filter((x) => x !== p);
          onChange({ ...filters, provider: rem });
        },
      });
    });
  }

  // Channel
  if (filters.channel && filters.channel.length) {
    filters.channel.forEach((c) => {
      activeChips.push({
        key: `chan-${c}`,
        label: `Channel: ${c}`,
        remove: () => {
          const rem = filters.channel!.filter((x) => x !== c);
          onChange({ ...filters, channel: rem });
        },
      });
    });
  }

  // EventId
  if (filters.eventId && filters.eventId.length) {
    filters.eventId.forEach((eid) => {
      activeChips.push({
        key: `eid-${eid}`,
        label: `EventID: ${eid}`,
        remove: () => {
          const rem = filters.eventId!.filter((x) => x !== eid);
          onChange({ ...filters, eventId: rem });
        },
      });
    });
  }

  // EventData
  if (filters.eventData && Object.keys(filters.eventData).length) {
    Object.entries(filters.eventData).forEach(([field, vals]) => {
      vals.forEach((v) => {
        activeChips.push({
          key: `ed-${field}-${v}`,
          label: `${field}: ${v}`,
          remove: () => {
            const currentVals = filters.eventData![field] ?? [];
            const newVals = currentVals.filter((x) => x !== v);
            const newEventData = { ...filters.eventData };
            if (newVals.length) newEventData[field] = newVals;
            else delete newEventData[field];
            onChange({ ...filters, eventData: newEventData });
          },
        });
      });
    });
  }

  // EventData Exclude
  if (
    filters.eventDataExclude &&
    Object.keys(filters.eventDataExclude).length
  ) {
    Object.entries(filters.eventDataExclude).forEach(([field, vals]) => {
      vals.forEach((v) => {
        activeChips.push({
          key: `edex-${field}-${v}`,
          label: `¬${field}: ${v}`,
          remove: () => {
            const currentVals = filters.eventDataExclude![field] ?? [];
            const newVals = currentVals.filter((x) => x !== v);
            const newEventDataEx = { ...filters.eventDataExclude };
            if (newVals.length) newEventDataEx[field] = newVals;
            else delete newEventDataEx[field];
            onChange({ ...filters, eventDataExclude: newEventDataEx });
          },
        });
      });
    });
  }

  // Compute facet counts: only supported when DuckDB has provided bucket counts.
  const facetCounts = useMemo(() => {
    if (!bucketCounts) {
      return {
        level: new Map<number, number>(),
        provider: new Map<string, number>(),
        channel: new Map<string, number>(),
        eventId: new Map<number, number>(),
      } as const;
    }

    const toMap = (
      obj?: Record<string, number>,
      numericKeys = false
    ): Map<string | number, number> => {
      const m = new Map<string | number, number>();
      if (!obj) return m;
      Object.entries(obj).forEach(([k, v]) => {
        const key = numericKeys ? Number(k) : k;
        m.set(key, v);
      });
      return m;
    };

    // Start maps with all keys from the full-file buckets so they never disappear.
    return {
      level: toMap(bucketCounts.level, true),
      provider: toMap(bucketCounts.provider),
      channel: toMap(bucketCounts.channel),
      eventId: toMap(bucketCounts.event_id, true),
    } as const;
  }, [bucketCounts]);

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
      logger.debug("toggleFilterValue", { facet, value });
      const current = (filters[facet] as (string | number)[] | undefined) ?? [];
      const exists = current.includes(value);
      const newVals = exists
        ? current.filter((v) => v !== value)
        : [...current, value];
      const next = { ...filters, [facet]: newVals } as FilterOptions;
      logger.debug("FilterSidebar onChange", { next });
      onChange(next);
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

  // constant moved above activeChips

  // Determine if any filters are currently active
  const hasActiveFilters = useMemo(() => {
    return (
      (filters.searchTerm && filters.searchTerm.trim() !== "") ||
      (filters.level && filters.level.length > 0) ||
      (filters.provider && filters.provider.length > 0) ||
      (filters.channel && filters.channel.length > 0) ||
      (filters.eventId && filters.eventId.length > 0) ||
      (filters.eventData && Object.keys(filters.eventData).length > 0) ||
      (filters.eventDataExclude &&
        Object.keys(filters.eventDataExclude).length > 0)
    );
  }, [filters]);

  return (
    <SidebarContainer>
      <Header>
        <span>Filters</span>
        {hasActiveFilters && !filtersDisabled && (
          <Button variant="subtle" size="small" onClick={() => onChange({})}>
            Clear
          </Button>
        )}
      </Header>

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
                    onChange={(e) =>
                      handleSearchChange("provider", e.target.value)
                    }
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
                    onChange={(e) =>
                      handleSearchChange("channel", e.target.value)
                    }
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
        </>
      )}
    </SidebarContainer>
  );
};
