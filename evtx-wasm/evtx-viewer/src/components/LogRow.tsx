import React from "react";
import styled from "styled-components";
import type { EvtxRecord, EvtxSystemData } from "../lib/types";

const iconStyle = (color: string) => ({ width: 16, height: 16, color });
import {
  Info20Regular as InfoCircle,
  Warning20Regular as Warning,
  DismissCircle20Regular as DismissCircle,
  ErrorCircle20Regular as ErrorBadge,
} from "@fluentui/react-icons";

const LEVEL_NAMES: Record<number, string> = {
  0: "Information", // LogAlways
  1: "Critical",
  2: "Error",
  3: "Warning",
  4: "Information",
  5: "Verbose",
};

const LEVEL_ICONS: Record<number, React.ReactNode> = {
  0: <InfoCircle style={iconStyle("#0078D4")} />,
  1: <DismissCircle style={iconStyle("#C42B1C")} />,
  2: <ErrorBadge style={iconStyle("#C42B1C")} />,
  3: <Warning style={iconStyle("#F7630C")} />,
  4: <InfoCircle style={iconStyle("#0078D4")} />,
  5: <InfoCircle style={iconStyle("#5C5C5C")} />,
};

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

const LevelCell = styled.div`
  display: flex;
  align-items: center;
  gap: 4px;
`;

// helper functions reused from table file
const formatDateTime = (systemTime?: string): string => {
  if (!systemTime) return "-";
  try {
    const date = new Date(systemTime);
    return date.toLocaleString("en-US", {
      month: "2-digit",
      day: "2-digit",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    });
  } catch {
    return systemTime;
  }
};

const getSystemData = (r: EvtxRecord): EvtxSystemData => r?.Event?.System || {};
const getEventId = (sys: EvtxSystemData): string => {
  const eventId = sys.EventID;
  if (typeof eventId === "object" && eventId !== null) {
    const obj = eventId as Record<string, unknown>;
    return String(obj["#text"] ?? "-");
  }
  return String(eventId ?? "-");
};
const getProvider = (sys: EvtxSystemData): string =>
  sys.Provider?.Name || sys.Provider_attributes?.Name || "-";
const getTimeCreated = (sys: EvtxSystemData): string =>
  sys.TimeCreated?.SystemTime || sys.TimeCreated_attributes?.SystemTime || "";
const getUserId = (sys: EvtxSystemData): string =>
  sys.Security?.UserID || sys.Security_attributes?.UserID || "-";

export interface LogRowProps {
  record: EvtxRecord;
  isEven: boolean;
  isSelected: boolean;
  onRowClick: (idx: number) => void;
  /** Global row index – used for keyboard navigation */
  rowIndex: number;
}

export const LogRow: React.FC<LogRowProps> = React.memo(
  ({ record: rec, isEven, isSelected, onRowClick, rowIndex }) => (
    <TR
      $isEven={isEven}
      $isSelected={isSelected}
      onClick={() => onRowClick(rowIndex)}
      data-row-idx={rowIndex}
    >
      <TD>
        <LevelCell>
          {LEVEL_ICONS[rec.Event?.System?.Level || 4] || LEVEL_ICONS[4]}
          <span>{LEVEL_NAMES[rec.Event?.System?.Level || 4]}</span>
        </LevelCell>
      </TD>
      <TD>{formatDateTime(getTimeCreated(getSystemData(rec)))}</TD>
      <TD>{getProvider(getSystemData(rec))}</TD>
      <TD>{getEventId(getSystemData(rec))}</TD>
      <TD>{getSystemData(rec).Task ?? "-"}</TD>
      <TD>{getUserId(getSystemData(rec))}</TD>
      <TD>{getSystemData(rec).Computer ?? "-"}</TD>
      <TD>{getSystemData(rec).Opcode ?? "-"}</TD>
      <TD>{getSystemData(rec).Keywords ?? "-"}</TD>
    </TR>
  )
);

LogRow.displayName = "LogRow";
