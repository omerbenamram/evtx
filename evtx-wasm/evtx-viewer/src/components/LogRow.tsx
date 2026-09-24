import React from "react";
import { styled } from "styled-components";
import type { TableColumn, TabularRow } from "../lib/types";
import { cellText, levelSeverity, type Severity } from "../lib/columns";
import { ROW_HEIGHT } from "../lib/rowWindow";
import type { TimeZone } from "../lib/timeZone";

export const TR = styled.tr<{ $isSelected: boolean; $severity: Severity | null }>`
  height: ${ROW_HEIGHT}px;
  background: ${({ theme, $isSelected }) =>
    $isSelected ? theme.colors.fill.selected : "transparent"};
  cursor: default;
  &:hover {
    background: ${({ theme, $isSelected }) =>
      $isSelected ? theme.colors.fill.selected : theme.colors.fill.hover};
  }
  > td:first-child {
    box-shadow: ${({ theme, $severity }) =>
      $severity ? `inset 3px 0 0 ${theme.colors.severity[$severity]}` : "none"};
  }
  @media (forced-colors: active) {
    &[aria-selected="true"] {
      background: Highlight;
      color: HighlightText;
    }
  }
`;

export const TD = styled.td<{ $align?: "right" }>`
  padding: 0 8px;
  box-sizing: border-box;
  height: ${ROW_HEIGHT}px;
  line-height: ${ROW_HEIGHT - 1}px;
  text-align: ${({ $align }) => $align ?? "left"};
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
`;

interface LogRowProps {
  record: TabularRow;
  isSelected: boolean;
  rowIndex: number;
  columns: TableColumn[];
  /** Time cells format in this zone; a change re-renders the memoized row. */
  timeZone: TimeZone;
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
  isSelected,
  onRowClick,
  onCellContextMenu,
  rowIndex,
  columns,
}: LogRowProps) {
  return (
    <TR
      $isSelected={isSelected}
      $severity={levelSeverity(record.level)}
      data-row-idx={rowIndex}
      aria-rowindex={rowIndex + 2}
      aria-selected={isSelected}
    >
      {columns.map((column) => {
        const text = cellText(column, record);
        return (
          <TD
            key={column.id}
            $align={column.align}
            data-column-id={column.id}
            title={text}
            onClick={() => onRowClick(rowIndex, column.id)}
            onContextMenu={(event) => onCellContextMenu(rowIndex, column, record, event)}
          >
            {column.accessor ? column.accessor(record) : text}
          </TD>
        );
      })}
      <td aria-hidden="true" onClick={() => onRowClick(rowIndex, columns[0]?.id ?? "")} />
    </TR>
  );
});
