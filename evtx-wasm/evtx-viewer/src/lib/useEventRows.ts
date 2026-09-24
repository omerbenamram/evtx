import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { DuckDbDataSource, PAGE_SIZE } from "./duckDbDataSource";
import { getRowWindow, getRowScrollTop } from "./rowWindow";
import { errorMessage } from "./types";

interface Options {
  dataSource: DuckDbDataSource;
  rowHeight: number;
  /** Receives each resolved match count; the table owns the only count query. */
  onCount: (count: number) => void;
}

const loadFailure = (source: DuckDbDataSource, attempt: number, cause: unknown) => ({
  source,
  attempt,
  message: errorMessage(cause, "Unable to load events."),
});

/** The DOM holds only viewport rows; the data source owns its bounded page cache. */
export function useEventRows({ dataSource, rowHeight, onCount }: Options) {
  const containerRef = useRef<HTMLElement>(null);
  const queryKey = dataSource.queryKey;
  const [count, setCount] = useState({ key: queryKey, value: 0 });
  // Only the first visible row is state: scrolling within a row needs no render.
  const [viewport, setViewport] = useState({ key: queryKey, row: 0, height: 0 });
  const [failure, setFailure] = useState<ReturnType<typeof loadFailure> | null>(null);
  const [retry, setRetry] = useState(0);
  const [, redraw] = useState(0);
  const totalRows = count.key === queryKey ? count.value : 0;
  const error =
    failure?.source === dataSource && failure.attempt === retry ? failure.message : null;

  useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    container.scrollTop = 0;
    const measure = () => {
      const row = Math.floor(container.scrollTop / rowHeight);
      const height = container.clientHeight;
      setViewport((previous) =>
        previous.key === queryKey && previous.row === row && previous.height === height
          ? previous
          : { key: queryKey, row, height },
      );
    };
    measure();
    container.addEventListener("scroll", measure, { passive: true });
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    return () => {
      container.removeEventListener("scroll", measure);
      observer.disconnect();
    };
  }, [queryKey, rowHeight]);

  useEffect(() => {
    let current = true;
    void dataSource
      .init()
      .then(() => {
        if (!current) return;
        setCount({ key: dataSource.queryKey, value: dataSource.getTotalRecords() });
        onCount(dataSource.getTotalRecords());
      })
      .catch((cause) => {
        if (current) setFailure(loadFailure(dataSource, retry, cause));
      });
    return () => {
      current = false;
    };
  }, [dataSource, retry, onCount]);

  const { items, totalHeight } = getRowWindow({
    count: totalRows,
    rowHeight,
    scrollTop: viewport.key === queryKey ? viewport.row * rowHeight : 0,
    height: viewport.height,
  });
  const firstPage = items.length ? Math.floor(items[0].index / PAGE_SIZE) : -1;
  const lastPage = items.length ? Math.floor(items[items.length - 1].index / PAGE_SIZE) : -1;

  useEffect(() => {
    if (firstPage < 0) return;
    let current = true;
    dataSource.setVisiblePages(firstPage, lastPage);
    const pages = Array.from({ length: lastPage - firstPage + 1 }, (_, index) => firstPage + index);
    void Promise.all(pages.map((page) => dataSource.getPage(page)))
      .then(() => {
        if (current) redraw((version) => version + 1);
      })
      .catch((cause) => {
        if (current) setFailure(loadFailure(dataSource, retry, cause));
      });
    return () => {
      current = false;
    };
  }, [dataSource, firstPage, lastPage, retry]);

  const scrollToIndex = useCallback(
    (index: number) => {
      const container = containerRef.current;
      if (!container) return;
      container.scrollTop = getRowScrollTop(index, {
        count: totalRows,
        rowHeight,
        scrollTop: container.scrollTop,
        height: container.clientHeight,
      });
    },
    [totalRows, rowHeight],
  );

  return {
    containerRef,
    items,
    totalHeight,
    scrollToIndex,
    totalRows,
    error,
    retry: () => {
      setFailure(null);
      setRetry((value) => value + 1);
    },
  };
}
