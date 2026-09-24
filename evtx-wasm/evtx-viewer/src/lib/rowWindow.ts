// Grid metrics (DESIGN.md "Grid"): 22px rows under a 24px header.
export const ROW_HEIGHT = 22;
export const TABLE_HEADER_HEIGHT = 24;
const OVERSCAN_ROWS = 20;

interface Viewport {
  count: number;
  rowHeight: number;
  scrollTop: number;
  height: number;
}

export function getRowWindow({ count, rowHeight, scrollTop, height }: Viewport) {
  const totalHeight = TABLE_HEADER_HEIGHT + count * rowHeight;
  if (count === 0 || height <= TABLE_HEADER_HEIGHT) return { items: [], totalHeight };
  const offset = Math.max(0, Math.min(scrollTop, totalHeight - height));
  const start = Math.max(0, Math.floor(offset / rowHeight) - OVERSCAN_ROWS);
  const end = Math.min(
    count,
    Math.ceil((offset + height - TABLE_HEADER_HEIGHT) / rowHeight) + OVERSCAN_ROWS,
  );
  const items = Array.from({ length: end - start }, (_, local) => {
    const index = start + local;
    const startPixel = TABLE_HEADER_HEIGHT + index * rowHeight;
    return { index, start: startPixel, end: startPixel + rowHeight };
  });
  return { items, totalHeight };
}

/** Keep the row between the sticky header and the bottom edge. */
export function getRowScrollTop(
  index: number,
  { count, rowHeight, scrollTop, height }: Viewport,
): number {
  if (!count) return 0;
  const row = Math.max(0, Math.min(index, count - 1));
  const rowTop = row * rowHeight;
  const rowBottom = TABLE_HEADER_HEIGHT + rowTop + rowHeight;
  const next =
    rowTop < scrollTop ? rowTop : rowBottom > scrollTop + height ? rowBottom - height : scrollTop;
  return Math.max(0, Math.min(next, TABLE_HEADER_HEIGHT + count * rowHeight - height));
}
