export interface SliceConfig {
  viewportStart: number;
  viewportHeight: number;
  chunkTop: number;
  chunkHeight: number;
  rowHeight: number;
  bufferRows: number;
  maxRows: number;
  recordCount: number;
}

/**
 * Compute the [startRow, endRow] (inclusive) within a chunk that intersect the
 * viewport plus buffer. Returns `null` if the chunk is entirely outside the
 * buffered viewport.
 */
export function computeSliceRows(cfg: SliceConfig): [number, number] | null {
  const {
    viewportStart,
    viewportHeight,
    chunkTop,
    chunkHeight,
    rowHeight,
    bufferRows,
    maxRows,
    recordCount,
  } = cfg;

  const bufferPx = bufferRows * rowHeight;
  const viewportEnd = viewportStart + viewportHeight;

  const chunkBottom = chunkTop + chunkHeight;

  // Entire chunk outside buffered viewport
  if (
    viewportEnd + bufferPx <= chunkTop ||
    viewportStart - bufferPx >= chunkBottom
  ) {
    return null;
  }

  // Intersection bounds in pixels within chunk
  const intersectTopPx =
    Math.max(viewportStart - bufferPx, chunkTop) - chunkTop;
  const intersectBottomPx =
    Math.min(viewportEnd + bufferPx, chunkBottom) - chunkTop;

  // No intersection if bottom is above top
  if (intersectBottomPx <= 0 || intersectTopPx >= chunkHeight) {
    return null;
  }

  let startRow = Math.floor(intersectTopPx / rowHeight);
  let endRow = Math.ceil(intersectBottomPx / rowHeight) - 1; // inclusive

  // Clamp to valid record indices
  startRow = Math.min(Math.max(0, startRow), recordCount - 1);
  endRow = Math.min(recordCount - 1, Math.max(startRow, endRow));

  // Enforce max rows window
  if (endRow - startRow + 1 > maxRows) {
    endRow = startRow + maxRows - 1;
  }

  // If after clamping we ended with an empty range, skip rendering
  if (startRow > endRow) {
    return null;
  }

  return [startRow, endRow];
}
