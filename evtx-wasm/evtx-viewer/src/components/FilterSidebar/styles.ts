import styled, { css } from "styled-components";

// ---------------- Shared styled-components for FilterSidebar ----------------

// ---- Container & layout ----
export const SidebarContainer = styled.div`
  display: flex;
  flex-direction: column;
  height: 100%;
  background: ${({ theme }) => theme.colors.background.secondary};
  padding-left: 3px; /* Account for the resize divider */
  /* Allow entire sidebar to scroll when content exceeds viewport */
  overflow-y: auto;
`;

export const ActiveFiltersBar = styled.div`
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  background: ${({ theme }) => theme.colors.background.tertiary};
`;

// ---- Filter chips ----
export const FilterChip = styled.span`
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

export const ChipRemoveBtn = styled.button`
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

// ---- Facet section ----

export const Section = styled.div``;

export const SectionHeader = styled.button<{ $isOpen: boolean }>`
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

  /* Bottom border when content is scrollable */
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
`;

export const Counts = styled.span`
  color: ${({ theme }) => theme.colors.text.tertiary};
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
