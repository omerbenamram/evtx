import React, { useCallback, useMemo, useState, useRef } from "react";
import styled from "styled-components";

import type { EvtxRecord, FilterOptions } from "../lib/types";
import { DuckDbDataSource } from "../lib/duckDbDataSource";
import { EventDetailsPane } from "./EventDetailsPane";
import { computeSliceRows, useChunkVirtualizer } from "../lib/virtualHelpers";
import { LogRow } from "./LogRow";
import { useRowNavigation } from "./useRowNavigation";
import { logger } from "../lib/logger";
import type { VirtualItem } from "@tanstack/react-virtual";

// ------------------------------------------------------------------
// Helper to build the <tr/> list for the current viewport.  Extracted out of
// JSX to keep the main component lean and readable.
// ------------------------------------------------------------------

interface GenerateRowsArgs {
  vItems: VirtualItem[];
  chunkRows: Map<number, EvtxRecord[]>;
  columnsCount: number;
  tableContainerRef: React.MutableRefObject<HTMLDivElement | null>;
  prefix: number[];
  selectedRow: number | null;
  handleRowClick: (idx: number) => void;
  ROW_HEIGHT: number;
  SLICE_BUFFER_ROWS: number;
  MAX_ROWS_PER_SLICE: number;
  virtualizerTotal: number;
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
  filters: FilterOptions;
  onRowSelect?: (rec: EvtxRecord) => void;
  /** Callback to add a new EventData field filter */
  onAddEventDataFilter?: (field: string, value: string) => void;
  /** Callback to exclude value */
  onExcludeEventDataFilter?: (field: string, value: string) => void;
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
  filters,
  onRowSelect,
  onAddEventDataFilter,
  onExcludeEventDataFilter,
}) => {
  const ROW_HEIGHT = 30; // single source of truth for row height
  const MAX_ROWS_PER_SLICE = 5000; // increased to load a few thousand rows at once
  const SLICE_BUFFER_ROWS = 2000; // expanded buffer size so more rows stay mounted

  // Predicate based on current filters (same rules as sidebar counts)
  const filterFn = useCallback<(rec: EvtxRecord) => boolean>(
    (rec) => {
      const opts = filters;
      if (!opts) return true;
      const sys = rec.Event?.System ?? {};

      if (
        opts.level &&
        opts.level.length &&
        !opts.level.includes(sys.Level ?? 4)
      )
        return false;
      if (opts.provider && opts.provider.length) {
        const prov = sys.Provider?.Name ?? sys.Provider_attributes?.Name ?? "";
        if (!opts.provider.includes(prov)) return false;
      }
      if (
        opts.channel &&
        opts.channel.length &&
        !opts.channel.includes(sys.Channel ?? "")
      )
        return false;
      if (
        opts.eventId &&
        opts.eventId.length &&
        !opts.eventId.includes(Number(sys.EventID ?? -1))
      )
        return false;

      const ev = (rec.Event?.EventData ?? {}) as Record<string, unknown>;
      if (opts.eventData) {
        for (const [k, allowed] of Object.entries(opts.eventData)) {
          if (allowed.length === 0) continue;
          const val = String(ev[k] ?? "");
          if (!allowed.includes(val)) return false;
        }
      }
      if (opts.eventDataExclude) {
        for (const [k, blocked] of Object.entries(opts.eventDataExclude)) {
          if (blocked.length === 0) continue;
          const val = String(ev[k] ?? "");
          if (blocked.includes(val)) return false;
        }
      }
      // searchTerm ignored for table performance; DuckDB handles counts only
      return true;
    },
    [filters]
  );

  const {
    containerRef: tableContainerRef,
    virtualizer,
    chunkRows,
    prefix,
    totalRows,
  } = useChunkVirtualizer({
    dataSource,
    rowHeight: ROW_HEIGHT,
    filterFn,
  });

  const getRowRecord = useCallback(
    (globalIdx: number): EvtxRecord | null => {
      // locate owning chunk via prefix (chunkCount is small → linear OK)
      let c = 0;
      while (c + 1 < prefix.length && prefix[c + 1] <= globalIdx) c++;
      const rows = chunkRows.get(c);
      return rows ? rows[globalIdx - prefix[c]] : null;
    },
    [chunkRows, prefix]
  );

  const columns = useMemo(
    () => [
      { header: "Level", width: 140 },
      { header: "Date & Time", width: 200 },
      { header: "Source", width: 200 },
      { header: "Event ID", width: 80 },
      { header: "Task", width: 100 },
      { header: "User", width: 140 },
      { header: "Computer", width: 180 },
      { header: "OpCode", width: 80 },
      { header: "Keywords", width: 160 },
    ],
    []
  );

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
                {columns.map((col) => (
                  <TH key={col.header} style={{ width: col.width }}>
                    {col.header}
                  </TH>
                ))}
              </tr>
            </THead>
            <TBody>
              {generateRows({
                vItems: virtualizer.getVirtualItems(),
                chunkRows,
                columnsCount: columns.length,
                tableContainerRef,
                prefix,
                selectedRow,
                handleRowClick,
                ROW_HEIGHT,
                SLICE_BUFFER_ROWS,
                MAX_ROWS_PER_SLICE,
                virtualizerTotal: virtualizer.getTotalSize(),
              })}
            </TBody>
          </Table>
        </div>
      </TableContainer>

      {selectedRecord && (
        <>
          <Divider onMouseDown={handleDividerMouseDown} />
          <EventDetailsPane
            record={selectedRecord}
            height={detailsHeight}
            onAddFilter={onAddEventDataFilter}
            onExcludeFilter={onExcludeEventDataFilter}
          />
        </>
      )}
    </Container>
  );
};
