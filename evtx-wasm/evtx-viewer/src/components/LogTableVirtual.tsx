import React, { useCallback, useEffect, useMemo, useState, useRef } from "react";
import { styled, useTheme } from "styled-components";
import {
  ArrowDown16Filled,
  ArrowSort16Regular,
  ArrowUp16Filled,
  Filter16Filled,
  Filter16Regular,
} from "@fluentui/react-icons";
import { errorMessage, type EvtxRecord, type TableColumn, type TabularRow } from "../lib/types";
import { DuckDbDataSource, type RowSort } from "../lib/duckDbDataSource";
import { autoColumnWidth, FALLBACK_COLUMN_WIDTH } from "../lib/columns";
import { MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH } from "../state/columns/columnsSlice";
import { ResizeHandle } from "./Windows/ResizeHandle";
import { EventDetailsPane } from "./EventDetailsPane";
import { useEventRows } from "../lib/useEventRows";
import { ROW_HEIGHT, TABLE_HEADER_HEIGHT } from "../lib/rowWindow";
import { useEvtxMetaState, useGlobalDispatch } from "../state/store";
import { updateEvtxMeta } from "../state/evtx/evtxSlice";
import { LogRow, TD } from "./LogRow";
import { TitleBand } from "./EventDetailsPane";
import { ColumnFilterMenu, type FilterValue } from "./ColumnFilterMenu";
import { timeZoneLabel, useTimeZone } from "../lib/timeZone";
import { Button, ContextMenu, type ContextMenuItem } from "./Windows";
import { getColumnFacetCounts, MATCHED_COLUMN_ID } from "../lib/duckdb";
import { searchWords, withoutField, withTerm } from "../lib/searchQuery";
import { useFilters } from "../hooks/useFilters";
import { useColumns } from "../hooks/useColumns";
import {
  buildFacetConfigs,
  clearFacet,
  facetValues,
  formatFacetValue,
  isFiltered,
  toggleFacet,
} from "./FilterSidebar/facetUtils";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  overflow: hidden;
`;
const TableContainer = styled.section`
  flex: 1;
  min-height: 0;
  overflow: auto;
  position: relative;
  background: ${({ theme }) => theme.colors.surface.pane};
  scrollbar-width: thin;
  font-variant-numeric: tabular-nums;
  &:focus-visible {
    outline: 1px solid ${({ theme }) => theme.colors.focus};
    outline-offset: -1px;
  }
  /* With a selection, keyboard focus is drawn on the selected row instead. */
  &:focus-visible:has(tr[aria-selected="true"]) {
    outline: none;
  }
  td[data-column-id="${MATCHED_COLUMN_ID}"] {
    color: ${({ theme }) => theme.colors.text.secondary};
  }
  &:focus-visible tr[aria-selected="true"] {
    outline: 1px solid ${({ theme }) => theme.colors.focus};
    outline-offset: -1px;
  }
`;
const Table = styled.table`
  border-collapse: separate;
  border-spacing: 0;
  table-layout: fixed;
`;
const Band = styled(TitleBand)`
  gap: 24px;
  span + span {
    font-weight: 400;
  }
`;
const THead = styled.thead`
  position: sticky;
  top: 0;
  z-index: 10;
  background: ${({ theme }) => theme.colors.surface.pane};
`;
// Column separators are the resize handles' 1px lines.
const TH = styled.th`
  position: relative;
  padding: 0;
  height: ${TABLE_HEADER_HEIGHT}px;
  box-sizing: border-box;
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
  background: ${({ theme }) => theme.colors.surface.pane};
  font-weight: 600;
  white-space: nowrap;
`;
const HeaderControls = styled.div<{ $right: boolean }>`
  display: flex;
  flex-direction: ${({ $right }) => ($right ? "row-reverse" : "row")};
  align-items: center;
  height: ${TABLE_HEADER_HEIGHT - 1}px;
`;
const SortButton = styled.button<{ $right: boolean }>`
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: ${({ $right }) => ($right ? "row-reverse" : "row")};
  align-items: center;
  gap: 4px;
  height: 100%;
  padding: 0 ${({ $right }) => ($right ? "8px 0 4px" : "4px 0 8px")};
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: default;
  &:hover {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  &:focus-visible {
    outline: 1px solid ${({ theme }) => theme.colors.focus};
    outline-offset: -1px;
  }
  > span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  small {
    color: ${({ theme }) => theme.colors.text.tertiary};
    font-size: ${({ theme }) => theme.fontSize.secondary};
    font-weight: 400;
  }
`;
// Glyphs show on header hover/focus; an active sort or filter keeps a filled glyph.
const Glyph = styled.span<{ $active: boolean }>`
  display: inline-flex;
  flex: 0 0 16px;
  color: ${({ theme, $active }) => ($active ? theme.colors.text.primary : theme.colors.text.tertiary)};
  visibility: ${({ $active }) => ($active ? "visible" : "hidden")};
  ${TH}:hover &,
  ${TH}:focus-within & {
    visibility: visible;
  }
`;
const FilterButton = styled(Button).attrs({ variant: "subtle" })<{ $active: boolean }>`
  flex: 0 0 20px;
  min-width: 20px;
  height: 20px;
  margin: 0 2px;
  padding: 0;
  color: ${({ theme, $active }) => ($active ? theme.colors.accent.rest : theme.colors.text.secondary)};
  visibility: ${({ $active }) => ($active ? "visible" : "hidden")};
  ${TH}:hover &,
  ${TH}:focus-within &,
  &[aria-expanded="true"] {
    visibility: visible;
  }
`;
/** Grid-only columns: no sort, filter or header menu, not saved or listed in Choose columns. */
const NON_FILTERABLE = new Set([MATCHED_COLUMN_ID]);
const Notice = styled.div`
  padding: 12px;
  color: ${({ theme }) => theme.colors.text.primary};
  background: ${({ theme }) => theme.colors.surface.pane};
  button {
    margin-left: 8px;
  }
`;
interface Selection {
  source: DuckDbDataSource;
  index: number;
  key: string | null;
  record: EvtxRecord | null;
}
interface Props {
  dataSource: DuckDbDataSource;
  onManageColumns: () => void;
}

/** Event Viewer's order when the user has not chosen a sort. */
const DEFAULT_SORT: RowSort = { id: "time", desc: true };
/** Header click: ascending, descending, back to the default (time toggles newest/oldest). */
export function nextSort(current: RowSort | null, id: string): RowSort | null {
  const shown = current ?? DEFAULT_SORT;
  if (shown.id !== id) return { id, desc: false };
  if (!shown.desc) return id === DEFAULT_SORT.id ? null : { id, desc: true };
  return current ? null : { id, desc: false };
}
const AUTOSIZE_SAMPLE = 200;
/** Widest text as the grid lays it out (canvas ignores tabular-nums), in one layout pass. */
function measureTexts(font: string, texts: string[], header: boolean): number {
  const probe = document.createElement("div");
  probe.style.cssText = "position:absolute;visibility:hidden;width:max-content;white-space:pre;";
  // After cssText: the font shorthand would reset font-variant-numeric.
  probe.style.font = `${header ? 600 : 400} 12px ${font}`;
  probe.style.fontVariantNumeric = "tabular-nums";
  for (const text of new Set(texts))
    probe.append(Object.assign(document.createElement("div"), { textContent: text }));
  document.body.append(probe);
  const width = probe.getBoundingClientRect().width;
  probe.remove();
  return width;
}

export const LogTableVirtual: React.FC<Props> = ({ dataSource, onManageColumns }) => {
  const { filters, updateFilters } = useFilters();
  const { isLoading, fileInfo, totalRecords } = useEvtxMetaState();
  const timeZone = useTimeZone();
  const theme = useTheme();
  const dispatch = useGlobalDispatch();
  const { columns: dataColumns, resizeColumn, removeColumn } = useColumns();
  const words = useMemo(() => searchWords(filters.searchQuery), [filters.searchQuery]);
  // While the query has words, a leading "Matched in" column says where each row matched.
  const columns = useMemo<TableColumn[]>(
    () =>
      words.length
        ? [{ id: MATCHED_COLUMN_ID, header: "Matched in" }, ...dataColumns]
        : dataColumns,
    [words, dataColumns],
  );
  const [draftWidth, setDraftWidth] = useState<{ id: string; width: number } | null>(null);
  const [activeColumn, setActiveColumn] = useState<string | null>(null);
  // Content widths fill in columns the user has not sized; a resize commits to column state.
  const [autoWidths, setAutoWidths] = useState<Record<string, number>>({});
  const columnWidth = (column: TableColumn) =>
    draftWidth?.id === column.id
      ? draftWidth.width
      : (column.width ?? autoWidths[column.id] ?? FALLBACK_COLUMN_WIDTH);
  // null = the default sort (time, newest first).
  const [sort, setSort] = useState<RowSort | null>(null);
  const effectiveSort = sort ?? DEFAULT_SORT;
  const next = useMemo(() => dataSource.withSort(sort ?? DEFAULT_SORT), [dataSource, sort]);
  const [source, setSource] = useState(next);
  if (source !== next && next.replaces(source, isLoading)) {
    // Reusing before this render keeps shown rows mounted instead of flashing placeholders.
    next.reusePages(source);
    setSource(next);
  }
  // Handlers read the shown source here so import refreshes keep their identity.
  const shown = useRef(source);
  const selectionRequest = useRef(0);
  const filterRequest = useRef(0);
  useEffect(() => {
    // A new query invalidates in-flight selection and filter-menu requests.
    if (shown.current.queryKey !== source.queryKey) {
      selectionRequest.current++;
      filterRequest.current++;
    }
    shown.current = source;
  }, [source]);
  const publishCount = useCallback(
    (matchedCount: number) => dispatch(updateEvtxMeta({ matchedCount })),
    [dispatch],
  );
  const { containerRef, items, totalHeight, scrollToIndex, totalRows, error, retry } = useEventRows(
    {
      dataSource: source,
      rowHeight: ROW_HEIGHT,
      onCount: publishCount,
    },
  );
  const [selection, setSelection] = useState<Selection | null>(null);
  const [detailError, setDetailError] = useState<{ queryKey: string; message: string } | null>(
    null,
  );
  const currentError = detailError?.queryKey === source.queryKey ? detailError.message : null;
  const showError = useCallback(
    (message: string) => setDetailError({ queryKey: source.queryKey, message }),
    [source.queryKey],
  );
  const selected = selection?.source.queryKey === source.queryKey ? selection : null;
  const [detailsHeight, setDetailsHeight] = useState(220);
  const outerRef = useRef<HTMLDivElement>(null);
  const [detailsMax, setDetailsMax] = useState(500);

  useEffect(
    () => () => {
      selectionRequest.current++;
    },
    [],
  );
  useEffect(() => {
    const container = outerRef.current;
    if (!container) return;
    const observer = new ResizeObserver(() =>
      setDetailsMax(Math.max(100, container.clientHeight - 100)),
    );
    observer.observe(container);
    return () => observer.disconnect();
  }, []);
  const selectRow = useCallback(
    (index: number) => {
      const shownSource = shown.current;
      const request = ++selectionRequest.current;
      setDetailError(null);
      // Keep the previous record while the next one loads: dropping it unmounts the details
      // pane for a frame, the table grows into its space and renders extra rows there.
      setSelection((previous) => ({
        source: shownSource,
        index,
        key: null,
        record: previous?.record ?? null,
      }));
      void (async () => {
        try {
          const row = await shownSource.getRow(index);
          if (!row) throw new Error("This event is no longer available.");
          const record = await shownSource.getRecord(row.rowKey);
          if (!record) throw new Error("This event is no longer available.");
          if (request !== selectionRequest.current) return;
          setSelection({ source: shownSource, index, key: row.rowKey, record });
        } catch (cause) {
          if (request !== selectionRequest.current) return;
          setSelection(null);
          showError(errorMessage(cause, "Unable to load event details."));
        }
      })();
    },
    [showError],
  );

  // Appended rows can move a selected event in a sorted result. Its stable key keeps
  // details attached; the navigation index is re-ranked once per shown snapshot, which
  // a sorted view refreshes at most every IMPORT_REFRESH_MS while importing.
  const selectedKey = selected?.key;
  const selectionSource = selected?.source;
  useEffect(() => {
    if (!selectedKey || selectionSource === source) return;
    let current = true;
    void source
      .indexOf(selectedKey)
      .then((index) => {
        if (current && index !== null)
          setSelection((value) =>
            value?.key === selectedKey ? { ...value, source, index } : value,
          );
      })
      .catch((cause) => {
        if (current) showError(errorMessage(cause, "Unable to locate the selected event."));
      });
    return () => {
      current = false;
    };
  }, [source, selectedKey, selectionSource, showError]);

  const handleRowClick = useCallback(
    (index: number, columnId: string) => {
      setActiveColumn(columnId);
      containerRef.current?.focus({ preventScroll: true });
      selectRow(index);
    },
    [containerRef, selectRow],
  );
  const [menu, setMenu] = useState<{
    queryKey: string;
    items: ContextMenuItem[];
    position: { x: number; y: number };
    returnFocus: HTMLElement | null;
    label: string;
  } | null>(null);
  const currentMenu = menu?.queryKey === source.queryKey ? menu : null;
  const closeMenu = useCallback(() => {
    filterRequest.current++;
    setMenu(null);
  }, []);

  const [filterMenu, setFilterMenu] = useState<{
    queryKey: string;
    column: TableColumn;
    anchor: HTMLElement;
    values: FilterValue[] | null;
  } | null>(null);
  const currentFilterMenu = filterMenu?.queryKey === source.queryKey ? filterMenu : null;
  const closeFilterMenu = useCallback(() => {
    filterRequest.current++;
    setFilterMenu(null);
  }, []);
  const openFilterMenu = async (column: TableColumn, anchor: HTMLElement) => {
    const request = ++filterRequest.current;
    setMenu(null);
    setFilterMenu({ queryKey: source.queryKey, column, anchor, values: null });
    try {
      const counts = await getColumnFacetCounts(column.id, filters);
      if (request !== filterRequest.current) return;
      const facet = buildFacetConfigs(dataColumns).find((item) => item.id === column.id) ?? {
        id: column.id,
        label: column.header,
      };
      const values = counts.map(({ v, c }) => ({
        value: v,
        label: formatFacetValue(facet, v),
        count: Number(c),
      }));
      setFilterMenu((current) =>
        current?.column.id === column.id ? { ...current, values } : current,
      );
    } catch (cause) {
      if (request !== filterRequest.current) return;
      setFilterMenu(null);
      showError(errorMessage(cause, "Unable to load filter values."));
    }
  };

  const openHeaderMenu = (
    column: TableColumn,
    position: { x: number; y: number },
    returnFocus: HTMLElement | null,
  ) => {
    filterRequest.current++;
    const sorted = effectiveSort.id === column.id;
    const sortTo = (desc: boolean) => () =>
      setSort(
        column.id === DEFAULT_SORT.id && desc === DEFAULT_SORT.desc
          ? null
          : { id: column.id, desc },
      );
    const actions: ContextMenuItem[] = [
      {
        id: "sort-ascending",
        label: "Sort ascending",
        checked: sorted && !effectiveSort.desc,
        radio: true,
        onClick: sortTo(false),
      },
      {
        id: "sort-descending",
        label: "Sort descending",
        checked: sorted && effectiveSort.desc,
        radio: true,
        onClick: sortTo(true),
      },
      { id: "sort-separator", separator: true },
      {
        id: "filter",
        label: "Filter values…",
        disabled: !returnFocus,
        onClick: () => {
          if (returnFocus) void openFilterMenu(column, returnFocus);
        },
      },
      {
        id: "clear-filter",
        label: "Clear column filter",
        disabled: !isFiltered(filters, column.id),
        onClick: () => updateFilters((previous) => clearFacet(previous, column.id)),
      },
      { id: "column-separator", separator: true },
      {
        id: "reset-width",
        label: "Reset column width",
        onClick: () => resizeColumn(column.id, autoWidths[column.id] ?? FALLBACK_COLUMN_WIDTH),
      },
      {
        id: "hide-column",
        label: "Hide column",
        disabled: dataColumns.length < 2,
        onClick: () => removeColumn(column.id),
      },
      { id: "choose-columns", label: "Choose columns…", onClick: onManageColumns },
    ];
    setMenu({
      queryKey: source.queryKey,
      items: actions,
      position,
      returnFocus,
      label: `${column.header} column`,
    });
  };

  const copyValue = useCallback(
    async (value: string) => {
      try {
        await navigator.clipboard.writeText(value);
      } catch (cause) {
        showError(errorMessage(cause, "Unable to copy. Check clipboard permission."));
      }
    },
    [showError],
  );
  const copyEvent = useCallback(
    async (key: string) => {
      try {
        const record = await shown.current.getRecord(key);
        if (!record) throw new Error("This event is no longer available.");
        await navigator.clipboard.writeText(JSON.stringify(record, null, 2));
      } catch (cause) {
        showError(errorMessage(cause, "Unable to copy event details."));
      }
    },
    [showError],
  );
  const openRowMenu = useCallback(
    (row: TabularRow, column: TableColumn, position: { x: number; y: number }) => {
      filterRequest.current++;
      const value = String(row[column.id] ?? "");
      setMenu({
        queryKey: source.queryKey,
        position,
        returnFocus: containerRef.current,
        label: "Event actions",
        items: [
          {
            id: "copy-value",
            label: `Copy ${column.header} value`,
            onClick: () => {
              void copyValue(value);
            },
          },
          {
            id: "copy-event",
            label: "Copy event JSON",
            onClick: () => {
              void copyEvent(row.rowKey);
            },
          },
          {
            id: "filter-value",
            label: `Filter ${column.header} to this value`,
            disabled: NON_FILTERABLE.has(column.id),
            onClick: () =>
              updateFilters((previous) => ({
                ...previous,
                searchQuery: withTerm(
                  withoutField(previous.searchQuery ?? "", column.id),
                  column.id,
                  value,
                  false,
                ),
              })),
          },
        ],
      });
    },
    [source.queryKey, containerRef, copyValue, copyEvent, updateFilters],
  );
  const handleCellContextMenu = useCallback(
    (
      index: number,
      column: TableColumn,
      row: TabularRow,
      event: React.MouseEvent<HTMLTableCellElement>,
    ) => {
      event.preventDefault();
      handleRowClick(index, column.id);
      openRowMenu(row, column, { x: event.clientX, y: event.clientY });
    },
    [handleRowClick, openRowMenu],
  );
  const handleTableKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    // Until a sorted refresh re-ranks the selection, its index points into the old order.
    if (selected && selected.source !== source) return;
    if (event.target !== event.currentTarget) return;
    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
      event.preventDefault();
      const index = selected?.index;
      const column = columns.find((item) => item.id === activeColumn) ?? columns[0];
      if (index === undefined || !column) return;
      const row = source.peekRow(index);
      if (!row) return;
      const cell = containerRef.current?.querySelector<HTMLTableCellElement>(
        `[data-row-idx="${index}"] [data-column-id="${CSS.escape(column.id)}"]`,
      );
      const bounds = (cell ?? event.currentTarget).getBoundingClientRect();
      openRowMenu(row, column, { x: Math.max(8, bounds.left + 8), y: bounds.bottom });
      return;
    }
    if (event.altKey || event.ctrlKey || event.metaKey || !totalRows) return;
    let index: number;
    switch (event.key) {
      case "ArrowDown":
        index = Math.min(totalRows - 1, (selected?.index ?? -1) + 1);
        break;
      case "ArrowUp":
        index = Math.max(0, (selected?.index ?? 1) - 1);
        break;
      case "Home":
        index = 0;
        break;
      case "End":
        index = totalRows - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    selectRow(index);
    scrollToIndex(index);
  };
  // Size unsized columns once from the first loaded rows (later-added columns likewise).
  // Set during render, like `source` above, so the grid never paints the fallback widths twice.
  const unsized = columns.filter(
    (column) => column.width === undefined && autoWidths[column.id] === undefined,
  );
  const sample: TabularRow[] = [];
  for (let index = 0; unsized.length && index < Math.min(totalRows, AUTOSIZE_SAMPLE); index++) {
    const row = source.peekRow(index);
    if (row) sample.push(row);
  }
  if (sample.length) {
    const measure = (texts: string[], header: boolean) =>
      measureTexts(theme.fonts.body, texts, header);
    setAutoWidths((previous) => ({
      ...previous,
      ...Object.fromEntries(
        unsized.map((column) => [column.id, autoColumnWidth(column, sample, measure)]),
      ),
    }));
  }

  // The band names the log's channel ("Security"), as Event Viewer does; the file name until known.
  const [channel, setChannel] = useState<string | null>(null);
  const hasRows = totalRows > 0;
  useEffect(() => {
    if (!hasRows) return;
    let current = true;
    getColumnFacetCounts("channel", {}, 1)
      .then(([top]) => {
        if (current && top?.v) setChannel(top.v);
      })
      .catch(() => undefined); // ponytail: the file name stays as the fallback title.
    return () => {
      current = false;
    };
  }, [hasRows]);

  const spacer = (height: number) =>
    height > 0 && (
      <tr aria-hidden="true">
        <td aria-hidden="true" colSpan={columns.length + 1} style={{ height, padding: 0 }} />
      </tr>
    );
  const top = Math.max(0, (items[0]?.start ?? TABLE_HEADER_HEIGHT) - TABLE_HEADER_HEIGHT);
  const bottom = items.length ? Math.max(0, totalHeight - items[items.length - 1].end) : 0;
  const tableWidth = columns.reduce((width, column) => width + columnWidth(column), 0);
  const logName = channel ?? fileInfo?.fileName.replace(/\.evtx$/i, "");
  return (
    <Container ref={outerRef}>
      {logName && (
        <Band>
          <span>{logName}</span>
          <span>
            Number of events: {totalRecords.toLocaleString()}
            {totalRows !== totalRecords && ` · ${totalRows.toLocaleString()} shown`}
          </span>
        </Band>
      )}
      {(error || currentError) && (
        <Notice role="alert">
          {error || currentError}
          {error && <Button onClick={retry}>Retry</Button>}
        </Notice>
      )}
      <TableContainer
        ref={containerRef}
        tabIndex={0}
        onKeyDown={handleTableKeyDown}
        aria-label="Events. Use arrow keys to select an event, Home or End to jump."
      >
        <Table
          aria-label="Event log"
          aria-rowcount={totalRows + 1}
          // The last, unsized column stretches so header and rows run edge to edge.
          style={{ width: `max(${tableWidth}px, 100%)` }}
        >
          <colgroup>
            {columns.map((col) => (
              <col key={col.id} style={{ width: columnWidth(col) }} />
            ))}
            <col />
          </colgroup>
          <THead>
            <tr>
              {columns.map((col) => {
                const sorted = effectiveSort.id === col.id;
                const filtered = isFiltered(filters, col.id);
                const right = col.align === "right";
                const upcoming = nextSort(sort, col.id) ?? DEFAULT_SORT;
                const zone = col.id === "time" ? timeZoneLabel(timeZone) : null;
                const shortZone = timeZone === "utc" ? "UTC" : "local";
                const fixed = NON_FILTERABLE.has(col.id);
                return (
                  <TH
                    key={col.id}
                    scope="col"
                    title={zone ? `${col.header} (${zone})` : undefined}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      if (fixed) return;
                      const invoker =
                        event.currentTarget.querySelector<HTMLButtonElement>("button");
                      openHeaderMenu(col, { x: event.clientX, y: event.clientY }, invoker);
                    }}
                    onKeyDown={(event) => {
                      if (
                        fixed ||
                        (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10"))
                      )
                        return;
                      event.preventDefault();
                      event.stopPropagation();
                      const bounds = event.currentTarget.getBoundingClientRect();
                      openHeaderMenu(
                        col,
                        { x: bounds.left, y: bounds.bottom },
                        event.target instanceof HTMLElement ? event.target : null,
                      );
                    }}
                    aria-sort={sorted ? (effectiveSort.desc ? "descending" : "ascending") : "none"}
                  >
                    <HeaderControls $right={right}>
                      <SortButton
                        type="button"
                        $right={right}
                        aria-label={`Sort by ${col.header} ${upcoming.desc ? "descending" : "ascending"}`}
                        disabled={fixed}
                        onClick={() => setSort((current) => nextSort(current, col.id))}
                      >
                        <span>
                          {col.header}
                          {zone && <small> ({shortZone})</small>}
                        </span>
                        {!fixed && (
                          <Glyph $active={sorted} aria-hidden="true">
                            {!sorted ? (
                              <ArrowSort16Regular />
                            ) : effectiveSort.desc ? (
                              <ArrowDown16Filled />
                            ) : (
                              <ArrowUp16Filled />
                            )}
                          </Glyph>
                        )}
                      </SortButton>
                      {!fixed && (
                        <FilterButton
                          $active={filtered}
                          icon={filtered ? <Filter16Filled /> : <Filter16Regular />}
                          aria-label={`Filter ${col.header}`}
                          aria-haspopup="true"
                          aria-expanded={currentFilterMenu?.column.id === col.id}
                          onClick={(event) => {
                            if (currentFilterMenu?.column.id === col.id) closeFilterMenu();
                            else void openFilterMenu(col, event.currentTarget);
                          }}
                        />
                      )}
                    </HeaderControls>
                    <ResizeHandle
                      label={`Resize ${col.header} column`}
                      orientation="vertical"
                      value={columnWidth(col)}
                      min={MIN_COLUMN_WIDTH}
                      max={MAX_COLUMN_WIDTH}
                      onResize={(width) => setDraftWidth({ id: col.id, width })}
                      onCommit={(width) => {
                        if (fixed) setAutoWidths((previous) => ({ ...previous, [col.id]: width }));
                        else resizeColumn(col.id, width);
                        setDraftWidth(null);
                      }}
                      style={{ position: "absolute", right: -3, top: 0, height: "100%", margin: 0 }}
                    />
                  </TH>
                );
              })}
              <TH aria-hidden="true" />
            </tr>
          </THead>
          <tbody>
            {spacer(top)}
            {items.map((item) => {
              const row = source.peekRow(item.index);
              return row ? (
                <LogRow
                  key={row.rowKey}
                  record={row}
                  rowIndex={item.index}
                  isSelected={Boolean(selected?.key && selected.key === row.rowKey)}
                  onRowClick={handleRowClick}
                  onCellContextMenu={handleCellContextMenu}
                  columns={columns}
                  timeZone={timeZone}
                  words={words}
                />
              ) : (
                <tr
                  key={`loading-${item.index}`}
                  aria-rowindex={item.index + 2}
                  style={{ height: ROW_HEIGHT }}
                >
                  <TD colSpan={columns.length + 1}>Loading event…</TD>
                </tr>
              );
            })}
            {spacer(bottom)}
          </tbody>
        </Table>
        {!totalRows && !error && <Notice as="output">No events match the current filters.</Notice>}
      </TableContainer>
      {selected && !selected.record && <Notice as="output">Loading event details…</Notice>}
      {selected?.record && (
        <>
          <ResizeHandle
            label="Resize event details"
            orientation="horizontal"
            value={Math.min(detailsHeight, detailsMax)}
            min={100}
            max={detailsMax}
            reverse
            onResize={setDetailsHeight}
          />
          <EventDetailsPane record={selected.record} height={Math.min(detailsHeight, detailsMax)} />
        </>
      )}
      {currentMenu && (
        <ContextMenu
          items={currentMenu.items}
          position={currentMenu.position}
          onClose={closeMenu}
          returnFocus={currentMenu.returnFocus}
          ariaLabel={currentMenu.label}
        />
      )}
      {currentFilterMenu && (
        <ColumnFilterMenu
          key={currentFilterMenu.column.id}
          anchor={currentFilterMenu.anchor}
          label={`Filter ${currentFilterMenu.column.header}`}
          values={currentFilterMenu.values}
          included={facetValues(filters, currentFilterMenu.column.id)}
          filtered={isFiltered(filters, currentFilterMenu.column.id)}
          onToggle={(value) =>
            updateFilters((previous) => toggleFacet(previous, currentFilterMenu.column.id, value))
          }
          onClear={() =>
            updateFilters((previous) => clearFacet(previous, currentFilterMenu.column.id))
          }
          onClose={closeFilterMenu}
        />
      )}
    </Container>
  );
};
