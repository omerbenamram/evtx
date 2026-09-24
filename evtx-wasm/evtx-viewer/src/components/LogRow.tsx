import React from "react";
import { styled } from "styled-components";
import type { TableColumn, TabularRow } from "../lib/types";

export const ROW_HEIGHT = 30;

const TR = styled.tr<{ $isSelected: boolean; $isEven: boolean }>`
  height: ${ROW_HEIGHT}px;
  background: ${({ theme, $isSelected, $isEven }) =>
    $isSelected
      ? theme.colors.selection.background
      : $isEven
        ? theme.colors.background.tertiary
        : theme.colors.background.secondary};
  cursor: default;
  &:hover {
    background: ${({ theme, $isSelected }) =>
      $isSelected ? theme.colors.selection.background : theme.colors.background.hover};
  }
`;

const TD = styled.td`
  padding: 4px 8px;
  box-sizing: border-box;
  height: ${ROW_HEIGHT}px;
  border-right: 1px solid ${({ theme }) => theme.colors.border.light};
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  &:last-child {
    border-right: none;
  }
`;

interface LogRowProps {
  record: TabularRow;
  isEven: boolean;
  isSelected: boolean;
  rowIndex: number;
  columns: TableColumn[];
  onRowClick: (index: number, columnId: string) => void;
  onCellContextMenu: (
    index: number,
    column: TableColumn,
    row: TabularRow,
    event: React.MouseEvent<HTMLTableCellElement>,
  ) => void;
}

export const LogRow = React.memo(function LogRow({
  record,
  isEven,
  isSelected,
  onRowClick,
  onCellContextMenu,
  rowIndex,
  columns,
}: LogRowProps) {
  return (
    <TR
      $isEven={isEven}
      $isSelected={isSelected}
      data-row-idx={rowIndex}
      aria-rowindex={rowIndex + 2}
      aria-selected={isSelected}
    >
      {columns.map((column) => (
        <TD
          key={column.id}
          data-column-id={column.id}
          title={String(record[column.id] ?? "")}
          onClick={() => onRowClick(rowIndex, column.id)}
          onContextMenu={(event) => onCellContextMenu(rowIndex, column, record, event)}
        >
          {column.accessor ? column.accessor(record) : String(record[column.id] ?? "-")}
        </TD>
      ))}
    </TR>
  );
});
