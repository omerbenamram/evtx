import React, { useCallback } from "react";
import styled from "styled-components";
import { type FilterOptions } from "../lib/types";

const Bar = styled.div`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.md};
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.lg};
  background: ${({ theme }) => theme.colors.background.tertiary};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
`;

const SearchInput = styled.input`
  flex: 1;
  padding: 4px 8px;
  font-size: ${({ theme }) => theme.fontSize.body};
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
`;

const LevelsContainer = styled.div`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.sm};
`;

const LevelLabel = styled.label`
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: ${({ theme }) => theme.fontSize.caption};
`;

// Local mapping of Windows Event levels to display names.
const LEVEL_NAME_MAP: Record<number, string> = {
  1: "Critical",
  2: "Error",
  3: "Warning",
  4: "Information",
};

interface FilterBarProps {
  value: FilterOptions;
  onChange: (newFilters: FilterOptions) => void;
}

export const FilterBar: React.FC<FilterBarProps> = ({ value, onChange }) => {
  const handleSearchChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      onChange({ ...value, searchTerm: e.target.value });
    },
    [value, onChange]
  );

  const handleLevelToggle = useCallback(
    (level: number) => {
      const current = value.level ?? [];
      const exists = current.includes(level);
      const newLevels = exists
        ? current.filter((l) => l !== level)
        : [...current, level];
      onChange({ ...value, level: newLevels });
    },
    [value, onChange]
  );

  return (
    <Bar>
      <SearchInput
        placeholder="Quick filter..."
        value={value.searchTerm ?? ""}
        onChange={handleSearchChange}
      />
      <LevelsContainer>
        {[1, 2, 3, 4].map((lvl) => (
          <LevelLabel key={lvl}>
            <input
              type="checkbox"
              checked={value.level?.includes(lvl) ?? false}
              onChange={() => handleLevelToggle(lvl)}
            />
            {LEVEL_NAME_MAP[lvl]}
          </LevelLabel>
        ))}
      </LevelsContainer>
    </Bar>
  );
};
