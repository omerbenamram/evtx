import { describe, expect, it } from "vitest";
import { getRowWindow, getRowScrollTop, ROW_HEIGHT, TABLE_HEADER_HEIGHT } from "../rowWindow";

const viewport = { count: 1_000_000, rowHeight: ROW_HEIGHT, scrollTop: 0, height: 636 };
const total = TABLE_HEADER_HEIGHT + viewport.count * ROW_HEIGHT;

describe("fixed event row window", () => {
  it("bounds the DOM independently of log size and includes both viewport edges", () => {
    expect([ROW_HEIGHT, TABLE_HEADER_HEIGHT]).toEqual([22, 24]);
    const top = getRowWindow(viewport);
    expect(top.items).toHaveLength(28 + 20);
    expect(top.items[0]).toEqual({
      index: 0,
      start: TABLE_HEADER_HEIGHT,
      end: TABLE_HEADER_HEIGHT + ROW_HEIGHT,
    });
    const middle = getRowWindow({ ...viewport, scrollTop: 300_015 });
    expect(middle.items[0]?.index).toBe(13637 - 20);
    expect(middle.items.at(-1)?.index).toBe(13665 + 20 - 1);
    expect(middle.totalHeight).toBe(total);
  });

  it("scrolls End to the final row and clamps oversized or empty windows", () => {
    const scrollTop = getRowScrollTop(viewport.count - 1, viewport);
    expect(scrollTop).toBe(total - viewport.height);
    const bottom = getRowWindow({ ...viewport, scrollTop });
    expect(bottom.items.at(-1)?.index).toBe(viewport.count - 1);
    expect(bottom.items.at(-1)?.end).toBe(scrollTop + viewport.height);
    expect(getRowWindow({ ...viewport, scrollTop: Number.MAX_SAFE_INTEGER })).toEqual(bottom);
    expect(getRowWindow({ ...viewport, count: 0 })).toEqual({
      items: [],
      totalHeight: TABLE_HEADER_HEIGHT,
    });
    expect(getRowWindow({ ...viewport, count: 2 }).items.map((item) => item.index)).toEqual([0, 1]);
  });

  it("leaves visible rows still, clears the sticky header when moving up, and reaches Home", () => {
    const scrolled = { ...viewport, scrollTop: 300 };
    expect(getRowScrollTop(15, scrolled)).toBe(300);
    expect(getRowScrollTop(9, scrolled)).toBe(9 * ROW_HEIGHT);
    expect(getRowScrollTop(40, scrolled)).toBe(300);
    expect(getRowScrollTop(41, scrolled)).toBe(TABLE_HEADER_HEIGHT + 42 * ROW_HEIGHT - 636);
    expect(getRowScrollTop(0, scrolled)).toBe(0);
    expect(getRowScrollTop(0, { ...viewport, count: 0 })).toBe(0);
  });
});
