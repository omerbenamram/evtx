import React from "react";
import styled from "styled-components";
// (EvtxRecord no longer needed)

// Removed unused icons and helper constants; cell rendering now delegated to
// the column accessor functions supplied via props.

const TR = styled.tr<{ $isSelected?: boolean; $isEven?: boolean }>`
  height: 30px; /* Keep in sync with ROW_HEIGHT in LogTableVirtual */
  background: ${({ theme, $isSelected, $isEven }) =>
    $isSelected
      ? theme.colors.selection.background
      : $isEven
      ? theme.colors.background.tertiary
      : theme.colors.background.secondary};
  cursor: pointer;

  &:hover {
    background: ${({ theme, $isSelected }) =>
      $isSelected
        ? theme.colors.selection.background
        : theme.colors.background.hover};
  }
`;

const TD = styled.td`
  padding: 4px 8px;
  border-right: 1px solid ${({ theme }) => theme.colors.border.light};
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;

  &:last-child {
    border-right: none;
  }
`;

export interface LogRowProps {
  record: Record<string, unknown>;
  isEven: boolean;
  isSelected: boolean;
  onRowClick: (idx: number) => void;
  /** Global row index – used for keyboard navigation */
  rowIndex: number;
  /** Columns definition – determines which cells are rendered */
  columns: import("../lib/types").TableColumn[];
}

export const LogRow: React.FC<LogRowProps> = React.memo(
  ({ record: rec, isEven, isSelected, onRowClick, rowIndex, columns }) => (
    <TR
      $isEven={isEven}
      $isSelected={isSelected}
      onClick={() => onRowClick(rowIndex)}
      data-row-idx={rowIndex}
    >
      {columns.map((col) => (
        <TD key={col.id}>
          {
            (col.accessor
              ? col.accessor(rec)
              : String(rec[col.id] ?? "-")) as React.ReactNode
          }
        </TD>
      ))}
    </TR>
  )
);

LogRow.displayName = "LogRow";
