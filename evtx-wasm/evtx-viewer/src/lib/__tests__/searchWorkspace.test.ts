// @vitest-environment node
import { describe, expect, it } from "vitest";
import { getEventDataFields, parseEvtxRecord } from "../types";
import {
  formatTerm,
  mergeQuery,
  parseSearchQuery,
  queryTerms,
  withoutField,
  withoutTerm,
  withTerm,
} from "../searchQuery";
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

const roundTrip = (id: string | undefined, value: string, exclude = false) => {
  const [{ values, ...term }] = queryTerms(formatTerm(id, value, exclude));
  expect([term.id, values, term.exclude]).toEqual([id, [value], exclude]);
};

describe("query editing", () => {
  it("serializes canonical fields and quotes values that need it, parsing them back", () => {
    expect(formatTerm("eventId", "4672", false)).toBe("event_id:4672");
    expect(formatTerm("level", "4", true)).toBe("-level:4");
    expect(formatTerm("eventData.SubjectUserName", "fsir", false)).toBe("@SubjectUserName:fsir");
    expect(formatTerm("eventData.a.b-c", "x", false)).toBe("@a.b-c:x");
    expect(formatTerm("eventData.Logon Type", "a b", false)).toBe('@"Logon Type":"a b"');
    expect(formatTerm("provider", 'say "hi"', false)).toBe('provider:"say \\"hi\\""');
    expect(formatTerm("user", "", false)).toBe('user:""');
    expect(formatTerm(undefined, "-x", false)).toBe('"-x"');
    for (const value of ["fsir", "a b", 'q"uote', "C:\\Windows\\cmd.exe", "x:y", "\\", "-1", ""])
      roundTrip("eventData.Path", value);
    roundTrip("eventData.a:b c", "v");
    roundTrip("time", "2016-10-06T00:00:00.000000Z", true);
    roundTrip(undefined, "a:b");
    roundTrip(undefined, "-dash");
    roundTrip(undefined, "C:\\Windows", true);
  });

  it('parses field:"" as not set, keeps field: an error, and keeps text numbers as typed', () => {
    expect(parseSearchQuery('user:"" -@Name:""')).toEqual({
      include: { user: [""] },
      exclude: { "eventData.Name": [""] },
    });
    expect(() => parseSearchQuery("user:")).toThrow(Error);
    expect(parseSearchQuery("@LogonId:0012 event_id:0012")).toEqual({
      include: { "eventData.LogonId": ["0012"], eventId: ["12"] },
    });
  });

  it("replaces opposite terms, never duplicates, and keeps the rest of the text", () => {
    const query = 'fsir  @SubjectUserName:fsir "logon type"';
    expect(withTerm(query, "eventData.SubjectUserName", "fsir", false)).toBe(query);
    expect(withTerm(query, "eventData.SubjectUserName", "fsir", true)).toBe(
      'fsir "logon type" -@SubjectUserName:fsir',
    );
    expect(withTerm("-level:4", "level", "4", false)).toBe("level:4");
    expect(withTerm("", "level", "4", true)).toBe("-level:4");
    expect(withoutTerm(query, "eventData.SubjectUserName", "fsir")).toBe('fsir "logon type"');
    expect(withoutTerm(query, undefined, "fsir")).toBe('@SubjectUserName:fsir "logon type"');
    expect(withoutTerm(query, undefined, "fsir", true)).toBe(query);
    expect(withoutField("event_id:1 x -event_id:2 level:4", "eventId")).toBe("x level:4");
  });

  it("merges a click edit into a draft the user is still typing", () => {
    expect(mergeQuery("level:4 fsi", "level:4", "level:4 event_id:1")).toBe(
      "level:4 fsi event_id:1",
    );
    expect(mergeQuery("level:4 fsi", "level:4", "")).toBe("fsi");
    expect(mergeQuery('fsi "open', "", "level:4")).toBeUndefined();
  });
});

describe("saved searches", () => {
  const view: SavedView = {
    name: "Failed logins",
    filters: {
      searchQuery: 'event_id:4625 @TargetUserName:Alice level:"" -provider:Security',
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
      { filters: { searchQuery: "level:abc" } },
      { filters: { searchQuery: "-time:yesterday" } },
      { filters: { searchQuery: 'provider:"open' } },
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
