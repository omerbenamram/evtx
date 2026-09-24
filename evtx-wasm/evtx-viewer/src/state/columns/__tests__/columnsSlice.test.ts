import { describe, expect, it } from "vitest";
import {
  columnsReducer,
  resizeColumn,
  removeColumn,
  addColumn,
  setColumns,
  MIN_COLUMN_WIDTH,
  MAX_COLUMN_WIDTH,
} from "../columnsSlice";
import { getDefaultColumns, buildEventDataColumn } from "../../../lib/columns";
import { readSavedViews, writeSavedViews, restoreColumns } from "../../../lib/savedViews";

describe("column widths", () => {
  it("keeps hidden custom columns and resized widths available after view changes", () => {
    const custom = buildEventDataColumn("TargetUserName");
    let state = columnsReducer(getDefaultColumns(), addColumn(custom));
    state = columnsReducer(state, resizeColumn(custom.id, 347));
    state = columnsReducer(state, removeColumn(custom.id));
    expect(state.find((column) => column.id === custom.id)).toMatchObject({
      hidden: true,
      width: 347,
    });
    state = columnsReducer(state, setColumns(getDefaultColumns()));
    state = columnsReducer(state, addColumn(custom));
    expect(state.find((column) => column.id === custom.id)).toMatchObject({
      hidden: false,
      width: 347,
    });
    expect(state.filter((column) => !column.hidden)).toHaveLength(getDefaultColumns().length + 1);
  });

  it("commits one column width, preserves the other columns, and ignores cancelled/no-op changes", () => {
    const columns = getDefaultColumns();
    const resized = columnsReducer(columns, resizeColumn("provider", 347));
    expect(resized.find((column) => column.id === "provider")?.width).toBe(347);
    expect(resized.find((column) => column.id === "eventId")).toBe(
      columns.find((column) => column.id === "eventId"),
    );
    expect(columnsReducer(resized, resizeColumn("provider", 347))).toBe(resized);
    expect(columnsReducer(resized, resizeColumn("provider", Number.NaN))).toBe(resized);
    expect(columnsReducer(resized, resizeColumn("missing", 300))).toBe(resized);
    expect(
      columnsReducer(columns, resizeColumn("provider", 1)).find(
        (column) => column.id === "provider",
      )?.width,
    ).toBe(MIN_COLUMN_WIDTH);
    expect(
      columnsReducer(columns, resizeColumn("provider", 10000)).find(
        (column) => column.id === "provider",
      )?.width,
    ).toBe(MAX_COLUMN_WIDTH);
  });

  it("round trips resized widths through saved views without changing column definitions", () => {
    const resized = columnsReducer(getDefaultColumns(), resizeColumn("provider", 347));
    const saved = writeSavedViews([
      {
        name: "Wide provider",
        filters: {},
        columns: resized.map(({ id, width }) => ({ id, width })),
      },
    ]);
    const [view] = readSavedViews(saved);
    expect(view).toBeDefined();
    if (!view) throw new Error("The saved view was not restored");
    const restored = restoreColumns(view.columns);
    expect(restored.find((column) => column.id === "provider")?.width).toBe(347);
    expect(restored.find((column) => column.id === "provider")?.header).toBe("Source");
  });

  it("keeps one column visible", () => {
    const one = getDefaultColumns().slice(0, 1);
    const first = one[0];
    if (!first) throw new Error("Missing default column");
    expect(columnsReducer(one, removeColumn(first.id))).toBe(one);
  });
});
