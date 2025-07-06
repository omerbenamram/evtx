import styled, { css } from "styled-components";

/**
 * Generic selectable row used in list UIs where a checkbox and label (and
 * optionally additional content) are shown in a flex row. The `$selected`
 * prop controls highlighted colour/weight.
 */
export const SelectableRow = styled.label<{ $selected?: boolean }>`
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
