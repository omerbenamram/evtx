import { styled, css } from "styled-components";

export const SelectableRow = styled.label<{ $selected?: boolean }>`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: ${({ theme }) => theme.spacing.sm};
  min-width: 0;
  min-height: 28px;
  padding: 3px 0;
  font-size: ${({ theme }) => theme.fontSize.caption};
  cursor: pointer;

  ${({ $selected, theme }) =>
    $selected &&
    css`
      color: ${theme.colors.accent.primary};
      font-weight: 600;
    `}
`;
