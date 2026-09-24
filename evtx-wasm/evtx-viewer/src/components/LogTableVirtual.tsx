import React, { useCallback, useEffect, useMemo, useState, useRef } from "react";
import { styled } from "styled-components";
import { ArrowDown16Regular, ArrowUp16Regular, Filter20Regular } from "@fluentui/react-icons";
import { errorMessage, type EvtxRecord, type TableColumn, type TabularRow } from "../lib/types";
import { DuckDbDataSource, type RowSort } from "../lib/duckDbDataSource";
import { getDefaultColumns } from "../lib/columns";
import { MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH } from "../state/columns/columnsSlice";
import { ResizeHandle } from "./Windows/ResizeHandle";
import { EventDetailsPane } from "./EventDetailsPane";
import { useEventRows } from "../lib/useEventRows";
import { TABLE_HEADER_HEIGHT } from "../lib/rowWindow";
import { useEvtxMetaState, useGlobalDispatch } from "../state/store";
import { updateEvtxMeta } from "../state/evtx/evtxSlice";
import { LogRow, ROW_HEIGHT } from "./LogRow";
import { Button, ContextMenu, type ContextMenuItem } from "./Windows";
import { getColumnFacetCounts } from "../lib/duckdb";
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
  &:focus-visible {
    outline: 2px solid ${({ theme }) => theme.colors.accent.primary};
    outline-offset: -2px;
  }
`;
const Table = styled.table`
  border-collapse: separate;
  border-spacing: 0;
  table-layout: fixed;
`;
const THead = styled.thead`
  position: sticky;
  top: 0;
  z-index: 10;
  background: ${({ theme }) => theme.colors.background.secondary};
`;
const TH = styled.th`
  position: relative;
  text-align: left;
  padding: 2px 8px 2px 4px;
  height: ${TABLE_HEADER_HEIGHT}px;
  box-sizing: border-box;
  border-right: 1px solid ${({ theme }) => theme.colors.border.light};
  border-bottom: 2px solid ${({ theme }) => theme.colors.border.medium};
  background: ${({ theme }) => theme.colors.background.secondary};
  font-weight: 600;
  white-space: nowrap;
  &:last-child {
    border-right: none;
  }
`;
const HeaderControls = styled.div`
  display: flex;
  align-items: center;
  gap: 2px;
`;
const HeaderButton = styled(Button).attrs({ size: "small", variant: "subtle" })<{
  $filter?: boolean;
}>`
  justify-content: ${({ $filter }) => ($filter ? "center" : "flex-start")};
  min-width: ${({ $filter }) => ($filter ? "28px" : "0")};
  flex: ${({ $filter }) => ($filter ? "0 0 28px" : "1")};
  padding: 2px 4px;
  span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
`;
const Notice = styled.div`
  padding: 12px;
  color: ${({ theme }) => theme.colors.text.primary};
  background: ${({ theme }) => theme.colors.background.secondary};
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

export const LogTableVirtual: React.FC<Props> = ({ dataSource, onManageColumns }) => {
  const { filters, updateFilters } = useFilters();
  const { isLoading } = useEvtxMetaState();
  const dispatch = useGlobalDispatch();
  const { columns, resizeColumn, removeColumn } = useColumns();
  const [draftWidth, setDraftWidth] = useState<{ id: string; width: number } | null>(null);
  const [activeColumn, setActiveColumn] = useState<string | null>(null);
  const columnWidth = (column: TableColumn) =>
    draftWidth?.id === column.id ? draftWidth.width : (column.width ?? 140);
  const [sort, setSort] = useState<RowSort | null>(null);
  const next = useMemo(() => dataSource.withSort(sort), [dataSource, sort]);
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
      setSelection({ source: shownSource, index, key: null, record: null });
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
    if (!sort || !selectedKey || selectionSource === source) return;
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
  }, [source, sort, selectedKey, selectionSource, showError]);

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

  const openFilterMenu = async (
    column: TableColumn,
    position: { x: number; y: number },
    returnFocus: HTMLElement | null,
  ) => {
    const request = ++filterRequest.current;
    const label = `Filter ${column.header}`;
    setMenu({
      queryKey: source.queryKey,
      position,
      returnFocus,
      label,
      items: [{ id: "loading", label: "Loading values…", disabled: true }],
    });
    try {
      const counts = await getColumnFacetCounts(column.id, filters);
      if (request !== filterRequest.current) return;
      const current = facetValues(filters, column.id);
      const filtered = isFiltered(filters, column.id);
      const facet = buildFacetConfigs(columns).find((item) => item.id === column.id) ?? {
        id: column.id,
        label: column.header,
      };
      setMenu({
        queryKey: source.queryKey,
        position,
        returnFocus,
        label,
        items: [
          {
            id: "clear",
            label: "Clear column filter",
            disabled: !filtered,
            onClick: () => updateFilters((previous) => clearFacet(previous, column.id)),
          },
          ...counts.map(({ v, c }) => ({
            id: `value:${v}`,
            label: `${formatFacetValue(facet, v)} (${c})`,
            checked: current.includes(v),
            onClick: () => updateFilters((previous) => toggleFacet(previous, column.id, v)),
          })),
        ],
      });
    } catch (cause) {
      if (request !== filterRequest.current) return;
      setMenu(null);
      showError(errorMessage(cause, "Unable to load filter values."));
    }
  };

  const openHeaderMenu = (
    column: TableColumn,
    position: { x: number; y: number },
    returnFocus: HTMLElement | null,
  ) => {
    filterRequest.current++;
    const actions: ContextMenuItem[] = [
      {
        id: "sort-ascending",
        label: "Sort ascending",
        onClick: () => setSort({ id: column.id, desc: false }),
      },
      {
        id: "sort-descending",
        label: "Sort descending",
        onClick: () => setSort({ id: column.id, desc: true }),
      },
      {
        id: "filter",
        label: "Filter values…",
        onClick: () => {
          void openFilterMenu(column, position, returnFocus);
        },
      },
      {
        id: "clear-filter",
        label: "Clear column filter",
        disabled: !isFiltered(filters, column.id),
        onClick: () => updateFilters((previous) => clearFacet(previous, column.id)),
      },
      {
        id: "reset-width",
        label: "Reset column width",
        onClick: () =>
          resizeColumn(
            column.id,
            getDefaultColumns().find((item) => item.id === column.id)?.width ?? 200,
          ),
      },
      {
        id: "hide-column",
        label: "Hide column",
        disabled: columns.length < 2,
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
            onClick: () =>
              updateFilters((previous) => ({
                ...previous,
                include: { ...previous.include, [column.id]: [value] },
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
    if (sort && selected && selected.source !== source) return;
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
  const spacer = (height: number) =>
    height > 0 && (
      <tr aria-hidden="true">
        <td aria-hidden="true" colSpan={columns.length} style={{ height, padding: 0 }} />
      </tr>
    );
  const top = Math.max(0, (items[0]?.start ?? TABLE_HEADER_HEIGHT) - TABLE_HEADER_HEIGHT);
  const bottom = items.length ? Math.max(0, totalHeight - items[items.length - 1].end) : 0;
  const tableWidth = columns.reduce((width, column) => width + columnWidth(column), 0);
  return (
    <Container ref={outerRef}>
      {(error || currentError) && (
        <Notice role="alert">
          {error || currentError}
          {error && (
            <Button size="small" onClick={retry}>
              Retry
            </Button>
          )}
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
          style={{ width: tableWidth, minWidth: tableWidth }}
        >
          <colgroup>
            {columns.map((col) => (
              <col key={col.id} style={{ width: columnWidth(col) }} />
            ))}
          </colgroup>
          <THead>
            <tr>
              {columns.map((col) => (
                <TH
                  key={col.id}
                  scope="col"
                  onContextMenu={(event) => {
                    event.preventDefault();
                    const invoker = event.currentTarget.querySelector<HTMLButtonElement>("button");
                    openHeaderMenu(col, { x: event.clientX, y: event.clientY }, invoker);
                  }}
                  onKeyDown={(event) => {
                    if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10"))
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
                  aria-sort={
                    sort?.id === col.id ? (sort.desc ? "descending" : "ascending") : "none"
                  }
                >
                  <HeaderControls>
                    <HeaderButton
                      type="button"
                      aria-label={
                        sort?.id === col.id && sort.desc
                          ? `Clear sort for ${col.header}`
                          : `Sort by ${col.header}${sort?.id === col.id ? " descending" : " ascending"}`
                      }
                      onClick={() =>
                        setSort((current) =>
                          current?.id === col.id
                            ? current.desc
                              ? null
                              : { id: col.id, desc: true }
                            : { id: col.id, desc: false },
                        )
                      }
                    >
                      <span>{col.header}</span>
                      {sort?.id === col.id &&
                        (sort.desc ? <ArrowDown16Regular /> : <ArrowUp16Regular />)}
                    </HeaderButton>
                    <HeaderButton
                      type="button"
                      $filter
                      active={isFiltered(filters, col.id)}
                      aria-label={`Filter ${col.header}`}
                      aria-haspopup="menu"
                      aria-expanded={currentMenu?.label === `Filter ${col.header}`}
                      onClick={(event) => {
                        const bounds = event.currentTarget.getBoundingClientRect();
                        void openFilterMenu(
                          col,
                          { x: bounds.left, y: bounds.bottom },
                          event.currentTarget,
                        );
                      }}
                    >
                      <Filter20Regular />
                    </HeaderButton>
                  </HeaderControls>
                  <ResizeHandle
                    label={`Resize ${col.header} column`}
                    orientation="vertical"
                    value={columnWidth(col)}
                    min={MIN_COLUMN_WIDTH}
                    max={MAX_COLUMN_WIDTH}
                    onResize={(width) => setDraftWidth({ id: col.id, width })}
                    onCommit={(width) => {
                      resizeColumn(col.id, width);
                      setDraftWidth(null);
                    }}
                    style={{ position: "absolute", right: -3, top: 0, height: "100%" }}
                  />
                </TH>
              ))}
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
                  isEven={item.index % 2 === 0}
                  isSelected={Boolean(selected?.key && selected.key === row.rowKey)}
                  onRowClick={handleRowClick}
                  onCellContextMenu={handleCellContextMenu}
                  columns={columns}
                />
              ) : (
                <tr
                  key={`loading-${item.index}`}
                  aria-rowindex={item.index + 2}
                  style={{ height: ROW_HEIGHT }}
                >
                  <td colSpan={columns.length} style={{ padding: "0 8px" }}>
                    Loading event…
                  </td>
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
    </Container>
  );
};
