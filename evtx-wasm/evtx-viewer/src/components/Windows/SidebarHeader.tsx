import { styled } from "styled-components";

// Pane caption: 28px, 12px semibold, fits one 24px subtle button on the right.
export const SidebarHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
  height: ${({ theme }) => theme.size.control};
  gap: ${({ theme }) => theme.spacing.sm};
  padding: 0 4px 0 ${({ theme }) => theme.spacing.sm};
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
  font-weight: 600;
  background: ${({ theme }) => theme.colors.surface.pane};
  position: sticky;
  top: 0;
  z-index: 5;
`;
