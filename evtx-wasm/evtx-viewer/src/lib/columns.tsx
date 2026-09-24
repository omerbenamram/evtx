import { styled } from "styled-components";
import { levelName, type TableColumn } from "./types";
import { EVENT_DATA_PREFIX } from "./columnSql";
import {
  Info20Regular as InfoCircle,
  Warning20Regular as Warning,
  DismissCircle20Regular as DismissCircle,
  ErrorCircle20Regular as ErrorBadge,
} from "@fluentui/react-icons";

const iconStyle = (color: string) => ({ width: 16, height: 16, color });

const LEVEL_ICONS = [
  <InfoCircle key="always" style={iconStyle("#0078D4")} />,
  <DismissCircle key="critical" style={iconStyle("#C42B1C")} />,
  <ErrorBadge key="error" style={iconStyle("#C42B1C")} />,
  <Warning key="warning" style={iconStyle("#F7630C")} />,
  <InfoCircle key="information" style={iconStyle("#0078D4")} />,
  <InfoCircle key="verbose" style={iconStyle("#5C5C5C")} />,
];

const LevelCell = styled.div`
  display: flex;
  align-items: center;
  gap: 4px;
`;

/** Local time with milliseconds, so the grid, details and time facet read the same. */
export function formatDateTime(systemTime?: string): string {
  if (!systemTime) return "-";
  const date = new Date(systemTime);
  if (Number.isNaN(date.getTime())) return systemTime;
  return date.toLocaleString("en-US", {
    month: "2-digit",
    day: "2-digit",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    fractionalSecondDigits: 3,
    hour12: false,
  });
}

export const getDefaultColumns = (): TableColumn[] => [
  {
    id: "level",
    header: "Level",
    accessor: (row) => (
      <LevelCell>
        {row.level !== null && LEVEL_ICONS[Number(row.level)]}
        <span>{levelName(row.level)}</span>
      </LevelCell>
    ),
    width: 140,
  },
  {
    id: "time",
    header: "Date & Time",
    accessor: (row) => formatDateTime(String(row.time ?? "")),
    width: 200,
  },
  { id: "provider", header: "Source", width: 200 },
  { id: "eventId", header: "Event ID", width: 112 },
  { id: "task", header: "Task", width: 100 },
  { id: "user", header: "User", width: 140 },
  { id: "computer", header: "Computer", width: 180 },
  { id: "opcode", header: "OpCode", width: 112 },
  { id: "keywords", header: "Keywords", width: 160 },
];

export const buildEventDataColumn = (field: string): TableColumn => ({
  id: `${EVENT_DATA_PREFIX}${field}`,
  header: field,
  width: 200,
});
