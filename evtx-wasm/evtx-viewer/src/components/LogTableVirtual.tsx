/* eslint-disable @typescript-eslint/no-explicit-any */
import React, { useCallback, useMemo, useState, useRef } from "react";
import styled from "styled-components";

import type { EvtxRecord, FilterOptions, TableColumn } from "../lib/types";
import { DuckDbDataSource } from "../lib/duckDbDataSource";
import { EventDetailsPane } from "./EventDetailsPane";
import { computeSliceRows, useChunkVirtualizer } from "../lib/virtualHelpers";
import { LogRow } from "./LogRow";
import { useRowNavigation } from "./useRowNavigation";
import { logger } from "../lib/logger";
import { ContextMenu, type ContextMenuItem } from "./Windows";
import { getColumnFacetCounts } from "../lib/duckdb";
import { useFilters } from "../hooks/useFilters";
import { useColumns } from "../hooks/useColumns";
import type { VirtualItem } from "@tanstack/react-virtual";
import { Filter20Regular } from "@fluentui/react-icons";

// ------------------------------------------------------------------
// Helper to build the <tr/> list for the current viewport.  Extracted out of
// JSX to keep the main component lean and readable.
// ------------------------------------------------------------------

interface GenerateRowsArgs {
  vItems: VirtualItem[];
  chunkRows: Map<number, any[]>; // generic rows
  columnsCount: number;
  tableContainerRef: React.MutableRefObject<HTMLDivElement | null>;
  prefix: number[];
  selectedRow: number | null;
  handleRowClick: (idx: number) => void;
  ROW_HEIGHT: number;
  SLICE_BUFFER_ROWS: number;
  MAX_ROWS_PER_SLICE: number;
  virtualizerTotal: number;
  columns: TableColumn[];
}

function generateRows({
  vItems,
  chunkRows,
  columnsCount,
  tableContainerRef,
  prefix,
  selectedRow,
  handleRowClick,
  ROW_HEIGHT,
  SLICE_BUFFER_ROWS,
  MAX_ROWS_PER_SLICE,
  virtualizerTotal,
  columns,
}: GenerateRowsArgs): React.ReactNode[] {
  const rows: React.ReactNode[] = [];
  if (vItems.length === 0) return rows;

  // Spacer before first visible chunk
  if (vItems[0].start > 0) {
    rows.push(
      <tr key="spacer-top">
        <td colSpan={columnsCount} style={{ height: vItems[0].start }} />
      </tr>
    );
  }

  vItems.forEach((vi, idx) => {
    const chunkIdx = vi.index;
    const records = chunkRows.get(chunkIdx);

    if (!records) {
      // Placeholder row while chunk loading
      rows.push(
        <tr key={`placeholder-${chunkIdx}`}>
          <td colSpan={columnsCount} style={{ height: vi.size }}>
            Loading chunk {chunkIdx}…
          </td>
        </tr>
      );
      return; // Continue to spacer between chunks
    }

    const startGlobal = prefix[chunkIdx] ?? 0;
    const scrollEl = tableContainerRef.current;
    const viewportStart = scrollEl?.scrollTop ?? 0;
    const viewportHeight = scrollEl?.clientHeight ?? 0;
    const viewportEnd = viewportStart + viewportHeight;

    const bufferPx = SLICE_BUFFER_ROWS * ROW_HEIGHT;
    const chunkTop = vi.start;
    const chunkBottom = vi.start + vi.size;

    // Skip if chunk outside buffered viewport
    if (
      viewportEnd + bufferPx <= chunkTop ||
      viewportStart - bufferPx >= chunkBottom
    ) {
      rows.push(
        <tr key={`skip-${chunkIdx}`}>
          <td colSpan={columnsCount} style={{ height: vi.size }} />
        </tr>
      );
      return;
    }

    const slice = computeSliceRows({
      viewportStart,
      viewportHeight,
      chunkTop,
      chunkHeight: vi.size,
      rowHeight: ROW_HEIGHT,
      bufferRows: SLICE_BUFFER_ROWS,
      maxRows: MAX_ROWS_PER_SLICE,
      recordCount: records.length,
    });

    if (!slice) {
      rows.push(
        <tr key={`skip-${chunkIdx}`}>
          <td colSpan={columnsCount} style={{ height: vi.size }} />
        </tr>
      );
      return;
    }

    const [sliceStartRow, sliceEndRow] = slice;

    logger.debug("renderSlice", { chunkIdx, sliceStartRow, sliceEndRow });

    // Top spacer inside chunk
    const topSpacerHeight = sliceStartRow * ROW_HEIGHT;
    if (topSpacerHeight > 0) {
      rows.push(
        <tr key={`top-pad-${chunkIdx}`}>
          <td colSpan={columnsCount} style={{ height: topSpacerHeight }} />
        </tr>
      );
    }

    // Actual visible rows
    records.slice(sliceStartRow, sliceEndRow + 1).forEach((rec, localIdx) => {
      const rowI = sliceStartRow + localIdx;
      const globalIdx = startGlobal + rowI;
      const isEven = globalIdx % 2 === 0;
      const isSelected = selectedRow === globalIdx;
      rows.push(
        <LogRow
          key={`r-${globalIdx}`}
          rowIndex={globalIdx}
          record={rec}
          isEven={isEven}
          isSelected={isSelected}
          onRowClick={handleRowClick}
          columns={columns}
        />
      );
    });

    // Bottom spacer inside chunk
    const bottomSpacerHeight = (records.length - sliceEndRow - 1) * ROW_HEIGHT;
    if (bottomSpacerHeight > 0) {
      rows.push(
        <tr key={`bot-pad-${chunkIdx}`}>
          <td colSpan={columnsCount} style={{ height: bottomSpacerHeight }} />
        </tr>
      );
    }

    // Spacer between this chunk and next
    const next = vItems[idx + 1];
    if (next) {
      const gap = next.start - (vi.start + vi.size);
      if (gap > 0) {
        rows.push(
          <tr key={`spacer-${chunkIdx}`}>
            <td colSpan={columnsCount} style={{ height: gap }} />
          </tr>
        );
      }
    }
  });

  // Spacer after last visible chunk
  const last = vItems[vItems.length - 1];
  rows.push(
    <tr key="spacer-bottom">
      <td
        colSpan={columnsCount}
        style={{ height: virtualizerTotal - (last.start + last.size) }}
      />
    </tr>
  );

  return rows;
}

interface LogTableVirtualProps {
  dataSource: DuckDbDataSource;
  onRowSelect?: (rec: EvtxRecord) => void;
}

// ---------- styled basics (slimmed down from original LogTable) ----------

const Container = styled.div`
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
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

const TH = styled.th`
  text-align: left;
  padding: 6px 8px;
  border-right: 1px solid ${({ theme }) => theme.colors.border.light};
  border-bottom: 2px solid ${({ theme }) => theme.colors.border.medium};
  background: ${({ theme }) => theme.colors.background.secondary};
  font-weight: 600;
  user-select: none;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;

  &:last-child {
    border-right: none;
  }
`;

const THInner = styled.div<{ $filtered?: boolean }>`
  display: flex;
  align-items: center;
  gap: 4px;
  cursor: pointer;
  ::after {
    content: "";
  }
  svg {
    opacity: ${({ $filtered }) => ($filtered ? 1 : 0.4)};
    color: ${({ theme, $filtered }) =>
      $filtered ? theme.colors.accent.primary : theme.colors.text.secondary};
  }
`;

// Add styled divider for resizing like original
const Divider = styled.div`
  height: 2px;
  cursor: row-resize;
  background: ${({ theme }) => theme.colors.border.light};
  flex-shrink: 0;
  transition: background 0.2s ease;
  &:hover {
    background: ${({ theme }) => theme.colors.accent.primary};
  }
`;

// ------------------------------------------------------------------------

export const LogTableVirtual: React.FC<LogTableVirtualProps> = ({
  dataSource,
  onRowSelect,
}) => {
  const { filters: currentFilters, setFilters } = useFilters();
  const { columns } = useColumns();

  const ROW_HEIGHT = 30; // single source of truth for row height
  const MAX_ROWS_PER_SLICE = 5000; // increased to load a few thousand rows at once
  const SLICE_BUFFER_ROWS = 2000; // expanded buffer size so more rows stay mounted

  const {
    containerRef: tableContainerRef,
    virtualizer,
    chunkRows,
    prefix,
    totalRows,
  } = useChunkVirtualizer({
    dataSource,
    rowHeight: ROW_HEIGHT,
  });

  const getRowRecord = useCallback(
    (globalIdx: number): EvtxRecord | null => {
      let c = 0;
      while (c + 1 < prefix.length && prefix[c + 1] <= globalIdx) c++;
      const rows = chunkRows.get(c);
      if (!rows) return null;
      const row: any = rows[globalIdx - prefix[c]];
      if (!row) return null;
      const raw = row["Raw"] as string | undefined;
      if (!raw) return null;
      try {
        return JSON.parse(raw);
      } catch {
        return null;
      }
    },
    [chunkRows, prefix]
  );

  const columnsMemo = useMemo(() => columns, [columns]);

  // Optional: expose scroll position for debugging / analytics
  const handleScroll = useCallback(() => {
    if (tableContainerRef.current) {
      logger.debug("scroll", {
        scrollTop: tableContainerRef.current.scrollTop,
      });
    }
  }, [tableContainerRef]);

  /* ------------------------------------------------------------------
   * Row selection handling
   * ------------------------------------------------------------------
   * Besides tracking the numeric row index for highlighting / keyboard
   * navigation, we also keep a reference to the *actual* EvtxRecord that
   * was selected.  This guarantees the EventDetailsPane can stay mounted
   * across filter changes because the record object itself does not change
   * when our virtualisation layers re-compute indices or re-order rows.
   */

  const [selectedRecord, setSelectedRecord] = useState<EvtxRecord | null>(null);

  // Wrap the optional onRowSelect prop so we can update our own state first
  const handleRowSelect = useCallback(
    (rec: EvtxRecord) => {
      setSelectedRecord(rec);
      if (onRowSelect) onRowSelect(rec);
    },
    [onRowSelect]
  );

  const { selectedRow, handleKeyDown, handleRowClick } = useRowNavigation({
    totalRows,
    getRowRecord,
    onRowSelect: handleRowSelect,
    scrollContainerRef: tableContainerRef,
    rowHeight: ROW_HEIGHT,
  });

  // We intentionally keep the selected record even if it no longer appears
  // in the filtered result set so the details pane remains visible while the
  // user tweaks filters.  It will be cleared automatically once the user
  // selects a different row or when a new file is loaded.

  // Height of the details pane (resizable via divider)
  const [detailsHeight, setDetailsHeight] = useState<number>(200);

  // outer container ref (for mouse move calculations during resize)
  const outerRef = useRef<HTMLDivElement>(null);

  // adjust divider
  const handleDividerMouseDown = useCallback(
    (e: React.MouseEvent<HTMLDivElement>) => {
      e.preventDefault();
      const startY = e.clientY;
      const startHeight = detailsHeight;

      const onMouseMove = (me: MouseEvent) => {
        if (!outerRef.current) return;
        const deltaY = me.clientY - startY;
        const newH = Math.max(100, startHeight - deltaY);
        setDetailsHeight(newH);
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

  // ---------------------------------------------
  // Header filter pop-over state
  // ---------------------------------------------
  const [filterMenu, setFilterMenu] = useState<{
    col: TableColumn;
    pos: { x: number; y: number };
    items: { v: string; c: number }[];
  } | null>(null);

  const openFilterMenu = async (col: TableColumn, e: React.MouseEvent) => {
    e.preventDefault();
    const counts = await getColumnFacetCounts(col, currentFilters);
    setFilterMenu({
      col,
      pos: { x: e.clientX, y: e.clientY },
      items: counts.map(({ v, c }) => ({ v: String(v ?? "-"), c })),
    });
  };

  const toggleValue = (val: string) => {
    setFilters((prev) => {
      const cur = prev.columnEquals?.[filterMenu!.col.id] ?? [];
      const exists = cur.includes(val);
      const nextVals = exists
        ? cur.filter((x: string) => x !== val)
        : [...cur, val];
      return {
        ...prev,
        columnEquals: {
          ...(prev.columnEquals ?? {}),
          [filterMenu!.col.id]: nextVals,
        },
      } as FilterOptions;
    });
  };

  return (
    <Container
      ref={outerRef}
      tabIndex={0}
      onKeyDown={handleKeyDown}
      style={{ outline: "none" }}
    >
      <TableContainer ref={tableContainerRef} onScroll={handleScroll}>
        <div
          style={{ height: virtualizer.getTotalSize(), position: "relative" }}
        >
          <Table>
            <THead>
              <tr>
                {columnsMemo.map((col) => (
                  <TH
                    key={col.id}
                    style={{ width: col.width, position: "relative" }}
                    onContextMenu={(e) => openFilterMenu(col, e)}
                  >
                    {(() => {
                      const isFiltered = Boolean(
                        currentFilters.columnEquals?.[col.id]?.length
                      );
                      return (
                        <THInner
                          $filtered={isFiltered}
                          onClick={(ev) => openFilterMenu(col, ev as any)}
                        >
                          {col.header}
                          <Filter20Regular />
                        </THInner>
                      );
                    })()}
                  </TH>
                ))}
              </tr>
            </THead>
            <TBody>
              {generateRows({
                vItems: virtualizer.getVirtualItems(),
                chunkRows,
                columnsCount: columnsMemo.length,
                tableContainerRef,
                prefix,
                selectedRow,
                handleRowClick,
                ROW_HEIGHT,
                SLICE_BUFFER_ROWS,
                MAX_ROWS_PER_SLICE,
                virtualizerTotal: virtualizer.getTotalSize(),
                columns: columnsMemo,
              })}
            </TBody>
          </Table>
        </div>
      </TableContainer>

      {selectedRecord && (
        <>
          <Divider onMouseDown={handleDividerMouseDown} />
          <EventDetailsPane record={selectedRecord} height={detailsHeight} />
        </>
      )}
      {filterMenu &&
        (() => {
          const current =
            currentFilters.columnEquals?.[filterMenu.col.id] ?? [];
          const menuItems: ContextMenuItem[] = [
            {
              id: "select-all",
              label: "(Select All)",
              onClick: () => {
                setFilters(
                  (prev) =>
                    ({
                      ...prev,
                      columnEquals: {
                        ...(prev.columnEquals ?? {}),
                        [filterMenu.col.id]: [],
                      },
                    } as FilterOptions)
                );
              },
            },
          ];
          filterMenu.items.forEach(({ v, c }) => {
            menuItems.push({
              id: v,
              label: `${v} (${c})`,
              icon: (
                <input type="checkbox" readOnly checked={current.includes(v)} />
              ),
              onClick: () => toggleValue(v),
            });
          });
          return (
            <ContextMenu
              items={menuItems}
              position={filterMenu.pos}
              onClose={() => setFilterMenu(null)}
            />
          );
        })()}
    </Container>
  );
};
