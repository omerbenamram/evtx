import { styled } from "styled-components";

export const SidebarHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
  min-height: 36px;
  gap: ${({ theme }) => theme.spacing.sm};
  padding: 4px ${({ theme }) => theme.spacing.md};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  font-weight: 600;
  background: ${({ theme }) => theme.colors.background.tertiary};
  position: sticky;
  top: 0;
  z-index: 5;
`;
