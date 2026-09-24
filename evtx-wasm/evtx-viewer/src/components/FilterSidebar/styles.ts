import { styled, css } from "styled-components";

export const SidebarContainer = styled.div`
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  min-width: 0;
  background: ${({ theme }) => theme.colors.background.secondary};
  padding-left: 3px; /* Account for the resize divider */
  overflow-y: auto;
`;

export const ActiveFiltersBar = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  background: ${({ theme }) => theme.colors.background.tertiary};
`;

export const FilterChip = styled.span`
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 0 0 0 6px;
  max-width: 100%;
  overflow-wrap: anywhere;
  background: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  font-size: ${({ theme }) => theme.fontSize.caption};
  color: ${({ theme }) => theme.colors.text.primary};
`;

export const SectionHeader = styled.button<{ $isOpen: boolean }>`
  display: flex;
  align-items: center;
  width: 100%;
  background: ${({ theme }) => theme.colors.background.secondary};
  border: none;
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  min-height: 32px;
  text-align: left;
  font: inherit;
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

export const SectionIcon = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  margin-right: ${({ theme }) => theme.spacing.sm};
`;

export const OptionsContainer = styled.div`
  max-height: 240px;
  overflow: auto;
  padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.md}
    ${({ theme }) => theme.spacing.md};

  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
`;

export const Counts = styled.span`
  color: ${({ theme }) => theme.colors.text.secondary};
  font-variant-numeric: tabular-nums;
  padding-left: 4px;
`;

export const Checkbox = styled.input.attrs({ type: "checkbox" })`
  width: 14px;
  height: 14px;
  margin: 0;
  cursor: pointer;
  accent-color: ${({ theme }) => theme.colors.accent.primary};
`;

export const OptionLabel = styled.span`
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;
