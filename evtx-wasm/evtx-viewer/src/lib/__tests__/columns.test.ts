import { describe, expect, it } from "vitest";
import { autoColumnWidth, getDefaultColumns, levelSeverity } from "../columns";
import { levelName } from "../types";
import { nextSort } from "../../components/LogTableVirtual";
import { MIN_COLUMN_WIDTH } from "../../state/columns/columnsSlice";

const column = (id: string) => {
  const found = getDefaultColumns().find((item) => item.id === id);
  if (!found) throw new Error(`missing ${id}`);
  return found;
};

const measure = (texts: string[]) => Math.max(0, ...texts.map((text) => text.length * 10));

describe("grid columns", () => {
  it("names level 0 Information and marks only warning, error and critical rows", () => {
    expect([0, 1, 2, 3, 4, 5].map(levelName)).toEqual([
      "Information",
      "Critical",
      "Error",
      "Warning",
      "Information",
      "Verbose",
    ]);
    expect([0, 1, 2, 3, "3", 4, 5, null].map(levelSeverity)).toEqual([
      null,
      "error",
      "error",
      "warning",
      "warning",
      null,
      null,
      null,
    ]);
  });

  it("sizes a column to its header or widest sampled cell, within bounds", () => {
    const rows = [
      { rowKey: "1", computer: "abc" },
      { rowKey: "2", computer: "a".repeat(20) },
    ];
    expect(autoColumnWidth(column("computer"), rows, measure)).toBe(200 + 17);
    expect(autoColumnWidth(column("computer"), [], measure)).toBe(80 + 44 + 17);
    expect(
      autoColumnWidth(column("computer"), [{ rowKey: "3", computer: "x".repeat(99) }], measure),
    ).toBe(360);
    expect(autoColumnWidth(column("eventId"), [], () => 0)).toBe(MIN_COLUMN_WIDTH);
    // The time header is sized with its zone suffix.
    expect(autoColumnWidth(column("time"), [], measure)).toBe(210 + 44 + 17);
    // The level icon adds its slot.
    expect(autoColumnWidth(column("level"), [{ rowKey: "4", level: 4 }], measure)).toBe(
      110 + 20 + 17,
    );
  });

  it("defaults to newest first and cycles header clicks back to the default", () => {
    expect(nextSort(null, "time")).toEqual({ id: "time", desc: false });
    expect(nextSort({ id: "time", desc: false }, "time")).toBeNull();
    expect(nextSort(null, "eventId")).toEqual({ id: "eventId", desc: false });
    expect(nextSort({ id: "eventId", desc: false }, "eventId")).toEqual({
      id: "eventId",
      desc: true,
    });
    expect(nextSort({ id: "eventId", desc: true }, "eventId")).toBeNull();
  });
});
