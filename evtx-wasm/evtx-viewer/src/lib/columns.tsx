// Column definitions and helpers for LogTableVirtual
// NOTE: This file is .tsx because the column accessors return JSX elements.

import React from "react";
import styled from "styled-components";
import type { TableColumn } from "./types";
import {
  Info20Regular as InfoCircle,
  Warning20Regular as Warning,
  DismissCircle20Regular as DismissCircle,
  ErrorCircle20Regular as ErrorBadge,
} from "@fluentui/react-icons";

// ---------------------------------------------------------------------------
// Shared cells & utilities
// ---------------------------------------------------------------------------

const iconStyle = (color: string) => ({ width: 16, height: 16, color });

const LEVEL_ICONS: Record<number, React.ReactNode> = {
  0: <InfoCircle style={iconStyle("#0078D4")} />,
  1: <DismissCircle style={iconStyle("#C42B1C")} />,
  2: <ErrorBadge style={iconStyle("#C42B1C")} />,
  3: <Warning style={iconStyle("#F7630C")} />,
  4: <InfoCircle style={iconStyle("#0078D4")} />,
  5: <InfoCircle style={iconStyle("#5C5C5C")} />,
};

const LEVEL_NAMES: Record<number, string> = {
  0: "LogAlways",
  1: "Critical",
  2: "Error",
  3: "Warning",
  4: "Information",
  5: "Verbose",
};

const LevelCell = styled.div`
  display: flex;
  align-items: center;
  gap: 4px;
`;

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

// ---------------------------------------------------------------------------
// Default/system columns
// ---------------------------------------------------------------------------

export const getDefaultColumns = (): TableColumn[] => [
  {
    id: "level",
    header: "Level",
    sqlExpr: "Level",
    accessor: (row) => {
      const level = (row["level"] as number | undefined) ?? 4;
      return (
        <LevelCell>
          {LEVEL_ICONS[level] || LEVEL_ICONS[4]}
          <span>{LEVEL_NAMES[level]}</span>
        </LevelCell>
      );
    },
    width: 140,
  },
  {
    id: "time",
    header: "Date & Time",
    // Our Evtx JSON encoder stores the timestamp under `Event.System.TimeCreated_attributes.SystemTime`
    // while others flatten it to Event.System.TimeCreated.SystemTime.
    // Use COALESCE so the column works for both shapes.
    // Keep as raw string; casting happens in queries that need TIMESTAMP/TIMESTAMPTZ
    sqlExpr: `coalesce(
      json_extract_string(Raw, '$.Event.System.TimeCreated_attributes.SystemTime'),
      json_extract_string(Raw, '$.Event.System.TimeCreated.SystemTime')
    )`,
    accessor: (row) => {
      const sysTime = row["time"] as string | undefined;
      return formatDateTime(sysTime);
    },
    width: 200,
  },
  {
    id: "provider",
    header: "Source",
    sqlExpr: "Provider",
    accessor: (row) => String(row["provider"] ?? "-"),
    width: 200,
  },
  {
    id: "eventId",
    header: "Event ID",
    sqlExpr: "EventID",
    accessor: (row) => String(row["eventId"] ?? "-"),
    width: 80,
  },
  {
    id: "task",
    header: "Task",
    sqlExpr: "json_extract_string(Raw, '$.Event.System.Task')",
    accessor: (row) => String(row["task"] ?? "-"),
    width: 100,
  },
  {
    id: "user",
    header: "User",
    sqlExpr: "json_extract_string(Raw, '$.Event.System.Security.UserID')",
    accessor: (row) => String(row["user"] ?? "-"),
    width: 140,
  },
  {
    id: "computer",
    header: "Computer",
    sqlExpr: "json_extract_string(Raw, '$.Event.System.Computer')",
    accessor: (row) => String(row["computer"] ?? "-"),
    width: 180,
  },
  {
    id: "opcode",
    header: "OpCode",
    sqlExpr: "json_extract_string(Raw, '$.Event.System.Opcode')",
    accessor: (row) => String(row["opcode"] ?? "-"),
    width: 80,
  },
  {
    id: "keywords",
    header: "Keywords",
    sqlExpr: "json_extract_string(Raw, '$.Event.System.Keywords')",
    accessor: (row) => String(row["keywords"] ?? "-"),
    width: 160,
  },
];

// ---------------------------------------------------------------------------
// Dynamic EventData column factory
// ---------------------------------------------------------------------------

export const buildEventDataColumn = (field: string): TableColumn => ({
  id: `eventData.${field}`,
  header: field,
  sqlExpr: `json_extract_string(Raw, '$.Event.EventData.${field}')`,
  accessor: (row) => String(row[`eventData.${field}`] ?? "-"),
  width: 200,
});
