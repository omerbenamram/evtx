// @vitest-environment node
import { describe, expect, it } from "vitest";
import { getEventDataFields, parseEvtxRecord } from "../types";
import { parseSearchQuery } from "../searchQuery";
import { readSavedViews, restoreColumns, writeSavedViews, type SavedView } from "../savedViews";
import { parseUtcRange, rangeForBuckets } from "../timeline";

describe("event search", () => {
  it("translates allowlisted fields, quoted values and plain text without SQL", () => {
    expect(
      parseSearchQuery(
        'event_id:4624 event_id:4625 provider:"O\'Brien Security" computer:HOST-1 channel:Security failed login',
      ),
    ).toEqual({
      include: {
        eventId: ["4624", "4625"],
        provider: ["O'Brien Security"],
        computer: ["HOST-1"],
        channel: ["Security"],
      },
      searchTerm: "failed login",
    });
    expect(parseSearchQuery("event_id:004624")).toEqual({ include: { eventId: ["4624"] } });
    expect(parseSearchQuery('"field:literal"')).toEqual({
      searchTerm: "field:literal",
    });
    expect(parseSearchQuery('provider:"a\\"b"')).toEqual({ include: { provider: ['a"b'] } });
    expect(parseSearchQuery(" ")).toEqual({});
    expect(parseSearchQuery("C:\\Windows\\cmd.exe http://x constructor:x")).toEqual({
      searchTerm: "C:\\Windows\\cmd.exe http://x constructor:x",
    });
  });

  it("rejects incomplete or unsupported queries instead of silently broadening results", () => {
    for (const query of [
      'provider:"unfinished',
      "event_id:",
      "event_id:12.5",
      "event_id:-1",
      "event_id:65536",
      "Provider:",
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

describe("UTC time selection", () => {
  it("keeps inclusive start and exclusive end for backward or single bucket selections", () => {
    const buckets = [
      { start: 1000, end: 2000, count: 2 },
      { start: 2000, end: 3000, count: 1 },
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
  it("parses inputs as UTC and refuses empty or reversed intervals", () => {
    expect(parseUtcRange("2026-09-24T10:00:00.123", "2026-09-24T11:00")).toEqual({
      start: new Date("2026-09-24T10:00:00.123Z"),
      end: new Date("2026-09-24T11:00:00Z"),
    });
    expect(() => parseUtcRange("2026-09-24T11:00", "2026-09-24T10:00")).toThrow(Error);
    expect(() => parseUtcRange("", "2026-09-24T10:00")).toThrow(Error);
    expect(() => parseUtcRange("2026-09-24T10:00", "2026-09-24T10:00")).toThrow(Error);
    expect(() => parseUtcRange("2026-02-30T10:00", "2026-03-01T10:00")).toThrow(Error);
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
