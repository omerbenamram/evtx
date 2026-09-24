import { styled } from "styled-components";
import {
  DismissCircle16Filled,
  ErrorCircle16Filled,
  Info16Filled,
  Warning16Filled,
} from "@fluentui/react-icons";
import { levelName, type TableColumn, type TabularRow } from "./types";
import { EVENT_DATA_PREFIX } from "./columnSql";
import { formatEventTime } from "./timeZone";
import { MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH } from "../state/columns/columnsSlice";

export type Severity = "error" | "warning";

/** Critical and error share the error color; only these rows get the severity edge. */
export function levelSeverity(level: TabularRow[string] | undefined): Severity | null {
  const value = Number(level);
  return value === 1 || value === 2 ? "error" : value === 3 ? "warning" : null;
}

const LEVEL_ICONS = new Map([
  [0, <Info16Filled key="0" />],
  [1, <DismissCircle16Filled key="1" />],
  [2, <ErrorCircle16Filled key="2" />],
  [3, <Warning16Filled key="3" />],
  [4, <Info16Filled key="4" />],
]);

const LevelCell = styled.span<{ $severity: Severity | null; $verbose: boolean }>`
  display: flex;
  align-items: center;
  gap: 4px;
  > svg,
  > i {
    flex: 0 0 16px;
    height: 16px;
    color: ${({ theme, $severity, $verbose }) =>
      $severity
        ? theme.colors.severity[$severity]
        : $verbose
          ? theme.colors.severity.verbose
          : theme.colors.text.secondary};
  }
  > span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
`;

/** The text a cell shows (and autosizing measures). */
export function cellText(column: TableColumn, row: TabularRow): string {
  const value = row[column.id];
  if (column.id === "level") return levelName(value);
  if (column.id === "time")
    return formatEventTime(value === null || value === undefined ? value : String(value));
  return value === null || value === undefined || value === "" ? "-" : String(value);
}

/** Level icon + name; the icon column sits in a 16px slot so names line up. */
export const LEVEL_ICON_WIDTH = 20;

export const getDefaultColumns = (): TableColumn[] => [
  {
    id: "level",
    header: "Level",
    accessor: (row) => (
      <LevelCell $severity={levelSeverity(row.level)} $verbose={Number(row.level) === 5}>
        {LEVEL_ICONS.get(Number(row.level)) ?? <i />}
        <span>{levelName(row.level)}</span>
      </LevelCell>
    ),
  },
  { id: "time", header: "Date and Time" },
  { id: "provider", header: "Source" },
  { id: "eventId", header: "Event ID", align: "right" },
  { id: "task", header: "Task", align: "right" },
  { id: "user", header: "User" },
  { id: "computer", header: "Computer" },
  { id: "opcode", header: "OpCode", align: "right" },
  { id: "keywords", header: "Keywords" },
];

export const buildEventDataColumn = (field: string): TableColumn => ({
  id: `${EVENT_DATA_PREFIX}${field}`,
  header: field,
});

/** Before content arrives, and for columns sized neither by the user nor by content. */
export const FALLBACK_COLUMN_WIDTH = 120;
// ponytail: content sizing stops at 360px so one long CommandLine does not push the grid
// off screen; the user can still drag wider (up to MAX_COLUMN_WIDTH).
const AUTO_MAX_WIDTH = Math.min(360, MAX_COLUMN_WIDTH);
const CELL_PADDING = 17; // 8px each side + the 1px header separator
const HEADER_GLYPHS = 44; // sort glyph + filter button

/**
 * Width that fits the header and the widest sampled cell. `measure` returns the widest of
 * its texts in pixels (header texts are semibold).
 */
export function autoColumnWidth(
  column: TableColumn,
  rows: TabularRow[],
  measure: (texts: string[], header: boolean) => number,
): number {
  // The time header also names the zone; size for the longer label.
  const header = column.id === "time" ? `${column.header} (local)` : column.header;
  const icon = column.id === "level" ? LEVEL_ICON_WIDTH : 0;
  const width = Math.max(
    measure([header], true) + HEADER_GLYPHS,
    measure(
      rows.map((row) => cellText(column, row)),
      false,
    ) + icon,
  );
  return Math.round(Math.max(MIN_COLUMN_WIDTH, Math.min(AUTO_MAX_WIDTH, width + CELL_PADDING)));
}
