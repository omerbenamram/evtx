import styled from "styled-components";

/**
 * Shared header bar used by sidebar-style panels such as FilterSidebar and
 * ColumnManager. It is intentionally minimal – just flex alignment, spacing,
 * and theme-aware colours. Consumers should not mutate its layout; instead
 * wrap it or compose additional elements inside.
 */
export const SidebarHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: ${({ theme }) => theme.spacing.sm};
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  font-weight: 600;
  background: ${({ theme }) => theme.colors.background.tertiary};
  position: sticky;
  top: 0;
  z-index: 5;
`;
