import { styled, css } from "styled-components";

export const SelectableRow = styled.label<{ $selected?: boolean }>`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: ${({ theme }) => theme.spacing.sm};
  min-width: 0;
  min-height: ${({ theme }) => theme.size.row};
  padding: 0;
  font-size: ${({ theme }) => theme.fontSize.body};
  cursor: pointer;

  ${({ $selected, theme }) =>
    $selected &&
    css`
      color: ${theme.colors.accent.rest};
      font-weight: 600;
    `}
`;
