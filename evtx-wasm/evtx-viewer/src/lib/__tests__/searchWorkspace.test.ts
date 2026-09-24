// @vitest-environment node
import { describe, expect, it } from "vitest";
import { getEventDataFields, parseEvtxRecord } from "../types";
import { parseSearchQuery } from "../searchQuery";
import { readSavedViews, restoreColumns, writeSavedViews, type SavedView } from "../savedViews";
import { formatTimeInput, parseTimeRange, rangeForBuckets } from "../timeline";

describe("event search", () => {
  it("maps fields to columns, - to exclude, @ to event data, and words to raw text", () => {
    expect(
      parseSearchQuery(
        'event_id:4624 ID:4625 source:"O\'Brien Security" -channel:Security @TargetUserName:bob level:error failed -"logon type"',
      ),
    ).toEqual({
      include: {
        eventId: ["4624", "4625"],
        provider: ["O'Brien Security"],
        "eventData.TargetUserName": ["bob"],
        level: ["2"],
      },
      exclude: { channel: ["Security"] },
      contains: ["failed"],
      notContains: ["logon type"],
    });
    expect(parseSearchQuery("event_id:004624 task:12544")).toEqual({
      include: { eventId: ["4624"], task: ["12544"] },
    });
    expect(parseSearchQuery('"field:literal" -')).toEqual({ contains: ["field:literal", "-"] });
    expect(parseSearchQuery('provider:"a\\"b"')).toEqual({ include: { provider: ['a"b'] } });
    expect(parseSearchQuery(" ")).toEqual({});
    expect(parseSearchQuery("C:\\Windows\\cmd.exe http://x constructor:x")).toEqual({
      contains: ["C:\\Windows\\cmd.exe", "http://x", "constructor:x"],
    });
  });

  it("rejects incomplete or invalid field values instead of silently broadening results", () => {
    for (const query of [
      'provider:"unfinished',
      "event_id:",
      "event_id:12.5",
      "event_id:-1",
      "Provider:",
      "level:loud",
      "task:abc",
    ]) {
      expect(() => parseSearchQuery(query)).toThrow(Error);
    }
  });
});

describe("saved searches", () => {
  const view: SavedView = {
    name: "Failed logins",
    filters: {
      searchQuery: "event_id:4625",
      include: { "eventData.TargetUserName": ["Alice"], level: [""] },
      exclude: { provider: ["Security"] },
      timeRange: {
        start: new Date("2026-09-24T10:00:00Z"),
        end: new Date("2026-09-24T11:00:00Z"),
      },
    },
    columns: [
      { id: "eventId", width: 90 },
      { id: "eventData.TargetUserName", width: 240 },
    ],
  };

  it("restores filter dates, column order and widths", () => {
    const [restored] = readSavedViews(writeSavedViews([view]));
    expect(restored).toEqual(view);
    const columns = restoreColumns(restored.columns);
    expect(columns.map(({ id, width }) => ({ id, width }))).toEqual(view.columns);
  });

  it("handles corrupted storage and rejects unsafe definitions and dates", () => {
    expect(readSavedViews("not json")).toEqual([]);
    expect(readSavedViews('{"version":2,"views":[]}')).toEqual([]);
    for (const patch of [
      { filters: { searchQuery: "event_id:wrong" } },
      { filters: { timeRange: { start: "bad", end: "bad" } } },
      { filters: { include: { unknown: ["x"] } } },
      { filters: { exclude: { constructor: ["x"] } } },
      { filters: { include: { level: ["abc"] } } },
      { filters: { exclude: { time: ["yesterday"] } } },
      { columns: [{ id: "eventData.name'); DROP TABLE logs; --" }] },
      { columns: [{ id: "eventId", width: -1 }] },
    ]) {
      expect(
        readSavedViews(JSON.stringify({ version: 1, views: [{ ...view, ...patch }] })),
      ).toEqual([]);
    }
    const [safe] = readSavedViews(
      JSON.stringify({
        version: 1,
        views: [{ ...view, columns: [{ id: "eventId", sqlExpr: "DROP TABLE logs" }] }],
      }),
    );
    expect(restoreColumns(safe.columns)[0]).not.toHaveProperty("sqlExpr");
  });
});

describe("time selection", () => {
  it("keeps inclusive start and exclusive end for backward or single bucket selections", () => {
    const buckets = [
      { start: 1000, end: 2000, count: 2, error: 0, warning: 0, information: 2 },
      { start: 2000, end: 3000, count: 1, error: 1, warning: 0, information: 0 },
    ];
    expect(rangeForBuckets(buckets, 1, 0)).toEqual({
      start: new Date(1000),
      end: new Date(3000),
    });
    expect(rangeForBuckets(buckets, 0, 0)).toEqual({
      start: new Date(1000),
      end: new Date(2000),
    });
    expect(rangeForBuckets([], 0, 0)).toBeUndefined();
  });
  it("parses inputs in the chosen zone and refuses empty or reversed intervals", () => {
    expect(parseTimeRange("2026-09-24T10:00:00.123", "2026-09-24 11:00", "utc")).toEqual({
      start: new Date("2026-09-24T10:00:00.123Z"),
      end: new Date("2026-09-24T11:00:00Z"),
    });
    expect(parseTimeRange("2026-09-24 10:00:00.5", "2026-09-24 11:00", "local")).toEqual({
      start: new Date(2026, 8, 24, 10, 0, 0, 500),
      end: new Date(2026, 8, 24, 11),
    });
    const instant = new Date("2026-09-24T10:00:00.123Z");
    for (const zone of ["utc", "local"] as const) {
      expect(
        parseTimeRange(formatTimeInput(instant, zone), "2027-01-01 00:00", zone).start,
      ).toEqual(instant);
    }
    expect(() => parseTimeRange("2026-09-24T11:00", "2026-09-24T10:00", "utc")).toThrow(Error);
    expect(() => parseTimeRange("", "2026-09-24T10:00", "utc")).toThrow(Error);
    expect(() => parseTimeRange("2026-09-24T10:00", "2026-09-24T10:00", "utc")).toThrow(Error);
    expect(() => parseTimeRange("2026-02-30T10:00", "2026-03-01T10:00", "local")).toThrow(Error);
  });
});

it("validates stored events while retaining additional fields and null attribute carriers", () => {
  const input = {
    Event: {
      System: {
        Provider: null,
        Provider_attributes: { Name: "Security" },
        EventID: 4624,
        Level: 0,
        Correlation: { ActivityID: "trace" },
      },
      EventData: { Data: { "#text": ["Alice", ""] }, Binary: "00" },
      Extra: { nested: [1, true, null] },
    },
  };
  const record = parseEvtxRecord(JSON.stringify(input));
  expect(record).toEqual(input);
  expect(getEventDataFields(record.Event.EventData ?? {})).toEqual([
    { name: "Data", value: '["Alice",""]' },
    { name: "Binary", value: "00" },
  ]);
  expect(() => parseEvtxRecord('{"Event":{"System":"broken"}}')).toThrow(/System/);
  // Forwarded (WEC) logs render numeric System values as strings.
  const forwarded = '{"Event":{"System":{"Level":"4","Task":"1","Version":"0"}}}';
  expect(parseEvtxRecord(forwarded).Event.System.Level).toBe("4");
});
