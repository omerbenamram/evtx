import React, { useState, useMemo, useRef, useCallback } from "react";
import styled from "styled-components";
import {
  useReactTable,
  getCoreRowModel,
  getSortedRowModel,
  type ColumnDef,
  flexRender,
  type SortingState,
  type ColumnResizeMode,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Info20Regular as InfoCircle,
  Warning20Regular as Warning,
  DismissCircle20Regular as DismissCircle,
  ErrorCircle20Regular as ErrorBadge,
  ChevronUp20Regular as ChevronUp,
  ChevronDown20Regular as ChevronDown,
} from "@fluentui/react-icons";
import { type EvtxRecord, type EvtxSystemData } from "../lib/types";

// Level constants matching Windows Event Viewer
const LEVEL_NAMES: Record<number, string> = {
  0: "Information", // LogAlways
  1: "Critical",
  2: "Error",
  3: "Warning",
  4: "Information",
  5: "Verbose",
};

const iconStyle = (color: string) => ({ width: 16, height: 16, color });

const LEVEL_ICONS: Record<number, React.ReactNode> = {
  0: <InfoCircle style={iconStyle("#0078D4")} />,
  1: <DismissCircle style={iconStyle("#C42B1C")} />,
  2: <ErrorBadge style={iconStyle("#C42B1C")} />,
  3: <Warning style={iconStyle("#F7630C")} />,
  4: <InfoCircle style={iconStyle("#0078D4")} />,
  5: <InfoCircle style={iconStyle("#5C5C5C")} />,
};

interface LogTableProps {
  data: EvtxRecord[];
  onRowSelect?: (record: EvtxRecord) => void;
}

const Container = styled.div`
  display: flex;
  flex-direction: column;
  height: 100%;
  background: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.light};
  font-family: ${({ theme }) => theme.fonts.body};
  font-size: ${({ theme }) => theme.fontSize.caption};
`;

const TableContainer = styled.div`
  flex: 1;
  overflow: auto;
  position: relative;
`;

const Table = styled.table`
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
`;

const THead = styled.thead`
  position: sticky;
  top: 0;
  z-index: 10;
  background: ${({ theme }) => theme.colors.background.secondary};
`;

const TBody = styled.tbody``;

const TR = styled.tr<{ $isSelected?: boolean; $isEven?: boolean }>`
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

const TH = styled.th<{ $isResizing?: boolean }>`
  text-align: left;
  padding: 6px 8px;
  border-right: 1px solid ${({ theme }) => theme.colors.border.light};
  border-bottom: 2px solid ${({ theme }) => theme.colors.border.medium};
  background: ${({ theme }) => theme.colors.background.secondary};
  font-weight: 600;
  user-select: none;

  /* Keep the column headers visible while scrolling */
  position: sticky;
  top: 0;
  z-index: 11; /* higher than the data rows */

  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;

  &:last-child {
    border-right: none;
  }

  ${({ $isResizing }) =>
    $isResizing &&
    `
    background: #E5F1FB;
  `}
`;

const THContent = styled.div`
  display: flex;
  align-items: center;
  gap: 4px;
`;

const TD = styled.td`
  padding: 4px 8px;
  border-right: 1px solid ${({ theme }) => theme.colors.border.light};
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;

  &:last-child {
    border-right: none;
  }
`;

const ColumnResizer = styled.div<{ $isResizing: boolean }>`
  position: absolute;
  right: 0;
  top: 0;
  height: 100%;
  width: 3px;
  cursor: col-resize;
  user-select: none;
  touch-action: none;

  ${({ $isResizing }) =>
    $isResizing &&
    `
    background: #0078D4;
  `}

  &:hover {
    background: ${({ theme }) => theme.colors.accent.primary};
  }
`;

const SortIcon = styled.span`
  display: inline-flex;
  align-items: center;
  margin-left: 4px;
`;

const DetailsPane = styled.div`
  height: 200px;
  border-top: 1px solid ${({ theme }) => theme.colors.border.medium};
  background: ${({ theme }) => theme.colors.background.secondary};
  padding: ${({ theme }) => theme.spacing.md};
  overflow-y: auto;
`;

const DetailSection = styled.div`
  margin-bottom: ${({ theme }) => theme.spacing.lg};
`;

const DetailTitle = styled.h3`
  font-size: ${({ theme }) => theme.fontSize.body};
  font-weight: 600;
  margin: 0 0 ${({ theme }) => theme.spacing.sm} 0;
  color: ${({ theme }) => theme.colors.text.primary};
`;

const DetailContent = styled.div`
  font-family: ${({ theme }) => theme.fonts.mono};
  font-size: ${({ theme }) => theme.fontSize.caption};
  color: ${({ theme }) => theme.colors.text.secondary};
  white-space: pre-wrap;
  word-break: break-word;
`;

const DetailRow = styled.div`
  display: flex;
  margin-bottom: 4px;
`;

const DetailLabel = styled.span`
  font-weight: 600;
  min-width: 120px;
  color: ${({ theme }) => theme.colors.text.secondary};
`;

const DetailValue = styled.span`
  color: ${({ theme }) => theme.colors.text.primary};
`;

const LevelCell = styled.div`
  display: flex;
  align-items: center;
  gap: 4px;
`;

// Helper functions
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

const getSystemData = (record: EvtxRecord): EvtxSystemData => {
  return record?.Event?.System || {};
};

const getEventId = (system: EvtxSystemData): string => {
  const eventId = system.EventID;
  if (typeof eventId === "object" && eventId !== null) {
    // The EventID can sometimes be an object when attributes are enabled.
    const obj = eventId as Record<string, unknown>;
    return String(obj["#text"] ?? "-");
  }
  return String(eventId ?? "-");
};

const getProvider = (system: EvtxSystemData): string => {
  return system.Provider?.Name || system.Provider_attributes?.Name || "-";
};

const getTimeCreated = (system: EvtxSystemData): string => {
  return (
    system.TimeCreated?.SystemTime ||
    system.TimeCreated_attributes?.SystemTime ||
    ""
  );
};

const getUserId = (system: EvtxSystemData): string => {
  return system.Security?.UserID || system.Security_attributes?.UserID || "-";
};

// Add styled divider for resizing
const Divider = styled.div`
  height: 2px; /* slimmer look */
  cursor: row-resize;
  background: ${({ theme }) => theme.colors.border.light};
  flex-shrink: 0;
  transition: background 0.2s ease;

  &:hover {
    background: ${({ theme }) => theme.colors.accent.primary};
  }
`;

export const LogTable: React.FC<LogTableProps> = ({ data, onRowSelect }) => {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [selectedRowId, setSelectedRowId] = useState<string | null>(null);
  // Height of the bottom details pane (in pixels)
  const [detailsHeight, setDetailsHeight] = useState<number>(200);
  const [columnResizeMode] = useState<ColumnResizeMode>("onChange");

  // Reference to the outer container to calculate drag movement
  const containerRef = useRef<HTMLDivElement>(null);
  const tableContainerRef = useRef<HTMLDivElement>(null);

  // Handle dragging of the divider to resize the details pane
  const handleDividerMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();

      const startY = e.clientY;
      const startHeight = detailsHeight;

      const onMouseMove = (moveEvent: MouseEvent) => {
        if (!containerRef.current) return;

        const deltaY = moveEvent.clientY - startY;
        const newHeight = Math.max(100, startHeight - deltaY); // Minimum 100px
        setDetailsHeight(newHeight);
      };

      const onMouseUp = () => {
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", onMouseUp);
      };

      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", onMouseUp);
    },
    [detailsHeight]
  );

  const selectedRecord = useMemo(() => {
    if (!selectedRowId) return null;
    const index = parseInt(selectedRowId);
    return data[index] || null;
  }, [selectedRowId, data]);

  const columns = useMemo<ColumnDef<EvtxRecord>[]>(
    () => [
      {
        id: "level",
        header: "Level",
        accessorFn: (row) => getSystemData(row).Level || 4,
        cell: ({ getValue }) => {
          const level = getValue() as number;
          return (
            <LevelCell>
              {LEVEL_ICONS[level] || LEVEL_ICONS[4]}
              <span>{LEVEL_NAMES[level] || "Information"}</span>
            </LevelCell>
          );
        },
        size: 120,
        minSize: 100,
        maxSize: 200,
      },
      {
        id: "dateTime",
        header: "Date and Time",
        accessorFn: (row) => getTimeCreated(getSystemData(row)),
        cell: ({ getValue }) => formatDateTime(getValue() as string),
        size: 180,
        minSize: 150,
        maxSize: 250,
      },
      {
        id: "source",
        header: "Source",
        accessorFn: (row) => getProvider(getSystemData(row)),
        size: 200,
        minSize: 100,
        maxSize: 400,
      },
      {
        id: "eventId",
        header: "Event ID",
        accessorFn: (row) => getEventId(getSystemData(row)),
        size: 80,
        minSize: 60,
        maxSize: 120,
      },
      {
        id: "task",
        header: "Task Category",
        accessorFn: (row) => getSystemData(row).Task || "-",
        size: 120,
        minSize: 80,
        maxSize: 200,
      },
      {
        id: "user",
        header: "User",
        accessorFn: (row) => getUserId(getSystemData(row)),
        size: 150,
        minSize: 100,
        maxSize: 300,
      },
      {
        id: "computer",
        header: "Computer",
        accessorFn: (row) => getSystemData(row).Computer || "-",
        size: 150,
        minSize: 100,
        maxSize: 300,
      },
      {
        id: "opcode",
        header: "OpCode",
        accessorFn: (row) => getSystemData(row).Opcode || "-",
        size: 80,
        minSize: 60,
        maxSize: 120,
      },
      {
        id: "keywords",
        header: "Keywords",
        accessorFn: (row) => getSystemData(row).Keywords || "-",
        size: 150,
        minSize: 100,
        maxSize: 300,
      },
    ],
    []
  );

  const table = useReactTable({
    data,
    columns,
    state: {
      sorting,
    },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    columnResizeMode,
  });

  const { rows } = table.getRowModel();

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => tableContainerRef.current,
    estimateSize: () => 30,
    overscan: 10,
  });

  const virtualRows = virtualizer.getVirtualItems();

  const handleRowClick = useCallback(
    (index: number) => {
      const rowId = String(index);
      setSelectedRowId(rowId);

      if (onRowSelect && data[index]) {
        onRowSelect(data[index]);
      }
    },
    [data, onRowSelect]
  );

  const renderEventData = (eventData: unknown): React.ReactNode => {
    if (!eventData) return "No event data";

    // Narrow the unknown type to the expected shape.
    const eventObj = eventData as Record<string, unknown>;

    if (eventObj["Data"]) {
      const rawData = eventObj["Data"] as unknown;
      const dataArray = Array.isArray(rawData) ? rawData : [rawData];
      return dataArray.map((rawItem, index: number) => {
        const item = rawItem as Record<string, unknown>;
        const name =
          (item["#attributes"] as Record<string, unknown> | undefined)?.Name ??
          `Data${index}`;
        const value = item["#text"] ?? "-";
        return (
          <DetailRow key={index}>
            <DetailLabel>{`${String(name)}:`}</DetailLabel>
            <DetailValue>{String(value)}</DetailValue>
          </DetailRow>
        );
      });
    }

    return JSON.stringify(eventData, null, 2);
  };

  return (
    <Container ref={containerRef}>
      <TableContainer ref={tableContainerRef}>
        {/* Spacer div ensures the scroll container has the full height of all rows */}
        <div
          style={{
            height: `${virtualizer.getTotalSize()}px`,
            position: "relative",
          }}
        >
          <Table>
            <THead>
              {table.getHeaderGroups().map((headerGroup) => (
                <tr key={headerGroup.id}>
                  {headerGroup.headers.map((header) => (
                    <TH
                      key={header.id}
                      style={{ width: header.getSize() }}
                      $isResizing={header.column.getIsResizing()}
                    >
                      <THContent
                        onClick={header.column.getToggleSortingHandler()}
                        style={{
                          cursor: header.column.getCanSort()
                            ? "pointer"
                            : "default",
                        }}
                      >
                        {flexRender(
                          header.column.columnDef.header,
                          header.getContext()
                        )}
                        {header.column.getIsSorted() && (
                          <SortIcon>
                            {header.column.getIsSorted() === "asc" ? (
                              <ChevronUp style={{ width: 12, height: 12 }} />
                            ) : (
                              <ChevronDown style={{ width: 12, height: 12 }} />
                            )}
                          </SortIcon>
                        )}
                      </THContent>
                      <ColumnResizer
                        onMouseDown={header.getResizeHandler()}
                        onTouchStart={header.getResizeHandler()}
                        $isResizing={header.column.getIsResizing()}
                      />
                    </TH>
                  ))}
                </tr>
              ))}
            </THead>
            <TBody>
              {/* Spacer row before visible items */}
              {virtualRows.length > 0 && virtualRows[0].start > 0 && (
                <tr>
                  <td
                    colSpan={columns.length}
                    style={{ height: virtualRows[0].start }}
                  />
                </tr>
              )}

              {virtualRows.map((virtualRow) => {
                const row = rows[virtualRow.index];
                const isSelected = selectedRowId === String(virtualRow.index);

                return (
                  <TR
                    key={row.id}
                    $isSelected={isSelected}
                    $isEven={virtualRow.index % 2 === 0}
                    onClick={() => handleRowClick(virtualRow.index)}
                    style={{ height: `${virtualRow.size}px` }}
                  >
                    {row.getVisibleCells().map((cell) => (
                      <TD
                        key={cell.id}
                        style={{ width: cell.column.getSize() }}
                      >
                        {flexRender(
                          cell.column.columnDef.cell,
                          cell.getContext()
                        )}
                      </TD>
                    ))}
                  </TR>
                );
              })}

              {/* Spacer row after visible items */}
              {virtualRows.length > 0 && (
                <tr>
                  <td
                    colSpan={columns.length}
                    style={{
                      height:
                        virtualizer.getTotalSize() -
                        virtualRows[virtualRows.length - 1].end,
                    }}
                  />
                </tr>
              )}
            </TBody>
          </Table>
        </div>
      </TableContainer>

      {selectedRecord && (
        <>
          {/* Divider for resizing */}
          <Divider onMouseDown={handleDividerMouseDown} />

          <DetailsPane style={{ height: `${detailsHeight}px` }}>
            <DetailSection>
              <DetailTitle>General</DetailTitle>
              <DetailRow>
                <DetailLabel>Log Name:</DetailLabel>
                <DetailValue>
                  {getSystemData(selectedRecord).Channel || "-"}
                </DetailValue>
              </DetailRow>
              <DetailRow>
                <DetailLabel>Source:</DetailLabel>
                <DetailValue>
                  {getProvider(getSystemData(selectedRecord))}
                </DetailValue>
              </DetailRow>
              <DetailRow>
                <DetailLabel>Event ID:</DetailLabel>
                <DetailValue>
                  {getEventId(getSystemData(selectedRecord))}
                </DetailValue>
              </DetailRow>
              <DetailRow>
                <DetailLabel>Level:</DetailLabel>
                <DetailValue>
                  {LEVEL_NAMES[getSystemData(selectedRecord).Level || 4]}
                </DetailValue>
              </DetailRow>
              <DetailRow>
                <DetailLabel>User:</DetailLabel>
                <DetailValue>
                  {getUserId(getSystemData(selectedRecord))}
                </DetailValue>
              </DetailRow>
              <DetailRow>
                <DetailLabel>Logged:</DetailLabel>
                <DetailValue>
                  {formatDateTime(
                    getTimeCreated(getSystemData(selectedRecord))
                  )}
                </DetailValue>
              </DetailRow>
              <DetailRow>
                <DetailLabel>Computer:</DetailLabel>
                <DetailValue>
                  {getSystemData(selectedRecord).Computer || "-"}
                </DetailValue>
              </DetailRow>
            </DetailSection>

            {!!selectedRecord.Event?.EventData && (
              <DetailSection>
                <DetailTitle>Event Data</DetailTitle>
                <DetailContent>
                  {renderEventData(selectedRecord.Event.EventData)}
                </DetailContent>
              </DetailSection>
            )}

            {!!selectedRecord.Event?.UserData && (
              <DetailSection>
                <DetailTitle>User Data</DetailTitle>
                <DetailContent>
                  {JSON.stringify(selectedRecord.Event.UserData, null, 2)}
                </DetailContent>
              </DetailSection>
            )}
          </DetailsPane>
        </>
      )}
    </Container>
  );
};
