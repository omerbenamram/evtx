import { useState, useCallback } from "react";
import type { EvtxRecord } from "../lib/types";

interface Params {
  totalRows: number;
  /** Retrieve record info for a global row index */
  getRowRecord: (idx: number) => EvtxRecord | null;
  /** Receives the newly-selected record (optional) */
  onRowSelect?: (rec: EvtxRecord) => void;
  /** Scrollable container that hosts the virtualised table */
  scrollContainerRef: React.MutableRefObject<HTMLDivElement | null>;
  /** Fixed pixel height of a *single* row */
  rowHeight: number;
}

export function useRowNavigation({
  totalRows,
  getRowRecord,
  onRowSelect,
  scrollContainerRef,
  rowHeight,
}: Params) {
  const [selectedRow, setSelectedRow] = useState<number | null>(null);

  const selectRow = useCallback(
    (index: number, ensureVisible = false) => {
      setSelectedRow(index);

      const rec = getRowRecord(index);
      if (rec && onRowSelect) onRowSelect(rec);

      if (ensureVisible) {
        const scrollEl = scrollContainerRef.current;
        if (!scrollEl) return;

        const viewportStart = scrollEl.scrollTop;
        const viewportEnd = viewportStart + scrollEl.clientHeight;

        const rowTop = index * rowHeight;
        const rowBottom = rowTop + rowHeight;

        if (rowTop < viewportStart) {
          scrollEl.scrollTop = rowTop;
        } else if (rowBottom > viewportEnd) {
          scrollEl.scrollTop = rowBottom - scrollEl.clientHeight;
        }
      }
    },
    [getRowRecord, onRowSelect, scrollContainerRef, rowHeight]
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      e.preventDefault();

      if (totalRows === 0) return;

      const currentIndex = selectedRow === null ? -1 : selectedRow;
      let newIndex = currentIndex;

      if (e.key === "ArrowDown") {
        newIndex = Math.min(totalRows - 1, currentIndex + 1);
      } else if (e.key === "ArrowUp") {
        newIndex = Math.max(0, currentIndex - 1);
      }

      if (newIndex !== currentIndex) {
        selectRow(newIndex, true);
      }
    },
    [selectedRow, totalRows, selectRow]
  );

  const handleRowClick = useCallback(
    (idx: number) => {
      selectRow(idx, false);
    },
    [selectRow]
  );

  return { selectedRow, handleKeyDown, handleRowClick, selectRow } as const;
}
