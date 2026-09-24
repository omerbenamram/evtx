import { readFileSync } from "node:fs";
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { EvtxWasmParser, initSync } from "../../wasm/evtx_wasm.js";
import { getEventDataFields, type ParsedQuery } from "../types";
import { openTestDatabase } from "./database";
import {
  buildWhere,
  countRecords,
  fetchRecordByKey,
  fetchRecords,
  fetchTabular,
  findRowIndex,
  getColumnFacetCounts,
  LOGS_TABLE_SQL,
  type QueryFilters,
} from "../duckdb";
import { eventDataExpression } from "../columnSql";
import { withTerm } from "../searchQuery";
import { formatTimeBucket, parseTimeRangeKey } from "../timeZone";
import { getTimeHistogram } from "../timeline";

let connection: Awaited<ReturnType<typeof openTestDatabase>>;
const query = async (sql: string) => connection.query(sql);
const count = (searchQuery: string) => countRecords({ searchQuery }, query);
const columns = [{ id: "eventId", header: "Event ID" }];
const matched = (searchQuery: string) =>
  fetchTabular(columns, { searchQuery }, 512, 0, null, query).then((rows) =>
    rows.map((row) => row.matchedIn),
  );
beforeAll(async () => {
  connection = await openTestDatabase();
});
afterAll(() => connection.close());
beforeEach(() => {
  connection.query(`DROP TABLE IF EXISTS logs; ${LOGS_TABLE_SQL}`);
  connection.query(`INSERT INTO logs (EventID, Level, Provider, Channel, TimeCreated, Computer, Raw) VALUES
    (4624, 4, 'O''Brien', 'Security', '2025-01-01 00:00:00', 'HOST', '{"Event":{"System":{"EventID":4624},"EventData":{"a.b''~/":"x''y"}}}'),
    (4625, 2, 'Other', 'Security', '2025-01-02 00:00:00', 'HOST', '{"Event":{"System":{"EventID":4625}}}'),
    (4624, NULL, 'Other', 'Application', NULL, 'OTHER', '{"Event":{"System":{"EventID":4624}}}')`);
});

describe("real DuckDB event queries", () => {
  it("projects cells without raw JSON, orders ties consistently, and resolves details and selected positions", async () => {
    const rows = await fetchTabular(columns, {}, 512, 0, { id: "eventId", desc: true }, query);
    expect(rows).toEqual([
      { eventId: 4625, rowKey: "1" },
      { eventId: 4624, rowKey: "0" },
      { eventId: 4624, rowKey: "2" },
    ]);
    expect(await fetchRecordByKey("1", query)).toMatchObject({
      Event: { System: { EventID: 4625 } },
    });
    expect(await fetchRecordByKey("99", query)).toBeNull();
    expect(await findRowIndex("2", {}, columns, { id: "eventId", desc: true }, query)).toBe(2);
    await expect(fetchRecordByKey("1 OR TRUE", query)).rejects.toThrow("Invalid event key");
    const quoted = await fetchTabular(
      [{ id: 'eventData.x"y', header: "x" }],
      {},
      1,
      0,
      null,
      query,
    );
    expect(quoted).toEqual([{ 'eventData.x"y': null, rowKey: "0" }]);
  });

  it("shows UTC time strings, sorts by the typed timestamp, and filters a cell value back", async () => {
    const time = [{ id: "time", header: "Time" }];
    const rows = await fetchTabular(time, {}, 512, 0, { id: "time", desc: true }, query);
    expect(rows).toEqual([
      { time: "2025-01-02T00:00:00.000000Z", rowKey: "1" },
      { time: "2025-01-01T00:00:00.000000Z", rowKey: "0" },
      { time: null, rowKey: "2" },
    ]);
    expect(await countRecords({ include: { time: [String(rows[0].time)] } }, query)).toBe(1);
    expect(await countRecords({ exclude: { time: [String(rows[0].time)] } }, query)).toBe(2);
    expect(await countRecords({ include: { time: [""] } }, query)).toBe(1);
    // A one-day span buckets by hour; values are [start, end) epoch-ms ranges.
    const [jan1, jan2] = [Date.parse("2025-01-01T00:00Z"), Date.parse("2025-01-02T00:00Z")];
    expect(await getColumnFacetCounts("time", {}, 20, query, "utc")).toEqual([
      { v: `${jan1}/${jan1 + 3_600_000}`, c: 1 },
      { v: `${jan2}/${jan2 + 3_600_000}`, c: 1 },
    ]);
  });

  it("applies escaped field paths, hidden column filters, structured search, and exclusive time ranges", async () => {
    const filters = {
      searchQuery: "computer:HOST event_id:4624",
      include: { provider: ["O'Brien"], "eventData.a.b'~/": ["x'y"] },
      timeRange: { start: new Date("2025-01-01T00:00:00Z"), end: new Date("2025-01-02T00:00:00Z") },
    };
    expect(await countRecords(filters, query)).toBe(1);
    expect(await fetchRecords(filters, 100, -1, query)).toHaveLength(1);
    // Export pages by row key: stable file order, no rescans of earlier pages.
    const first = await fetchRecords({ include: { channel: ["Security"] } }, 1, -1, query);
    const next = await fetchRecords({ include: { channel: ["Security"] } }, 1, first[0].key, query);
    expect([...first, ...next].map(({ key }) => key)).toEqual([0, 1]);
    expect(await fetchRecords({ include: { channel: ["Security"] } }, 1, 1, query)).toEqual([]);
    expect(eventDataExpression("a.b'~/")).toBe(
      "json_extract_string(Raw, '/Event/EventData/a.b''~0~1')",
    );
    expect(() => buildWhere({ include: { unknown: ["value"] } })).toThrow("Unknown column");
    expect(() => buildWhere({ exclude: { constructor: ["value"] } })).toThrow("Unknown column");
    expect(await countRecords({ exclude: { "eventData.missing": ["value"] } }, query)).toBe(3);
    // The details pane shows unnamed <Data> values; filtering on one finds its event.
    connection.query(
      `INSERT INTO logs (Raw) VALUES ('{"Event":{"System":{},"EventData":{"Data":{"#text":["a",""]},"Binary":null}}}')`,
    );
    const [data] = getEventDataFields({ Data: { "#text": ["a", ""] } });
    expect(
      await countRecords({ include: { [`eventData.${data.name}`]: [data.value] } }, query),
    ).toBe(1);
  });

  it("searches level names, exclusions, event data fields and backslashes in raw text", async () => {
    connection.query(
      String.raw`INSERT INTO logs (EventID, Level, Channel, Raw) VALUES (1, 2, 'System', '{"Event":{"EventData":{"Path":"C:\\Windows\\cmd.exe"}}}')`,
    );
    expect(await count(String.raw`C:\Windows\cmd.exe`)).toBe(1);
    expect(await count(String.raw`@Path:C:\Windows\cmd.exe`)).toBe(1);
    expect(await count("level:error")).toBe(2);
    expect(await count("-channel:Security")).toBe(2);
    expect(await count("event_id:4624 event_id:4625 channel:Security")).toBe(2);
    expect(await count("-4625")).toBe(3);
  });

  it("filters the same rows from helper-built query text as from column value lists", async () => {
    connection.query(`INSERT INTO logs (EventID, Level, Provider, UserID, Raw) VALUES
      (7, 0, '', NULL, '{"Event":{"System":{"EventID":7}}}'),
      (8, 4, 'a "b" c', 'S-1', '{"Event":{"System":{"EventID":8},"EventData":{"Logon Type":"x:y"}}}')`);
    const cases: [string, string, boolean][][] = [
      [["provider", "O'Brien", false]],
      [["level", "4", true]],
      [["level", "", false]],
      [
        ["level", "0", false],
        ["level", "", false],
      ],
      [["user", "", false]],
      [["user", "", true]],
      [
        ["provider", "", false],
        ["eventId", "7", true],
      ],
      [["provider", 'a "b" c', false]],
      [["eventData.a.b'~/", "x'y", false]],
      [["eventData.Logon Type", "x:y", false]],
      [
        ["eventData.Logon Type", "x:y", true],
        ["computer", "HOST", true],
      ],
    ];
    for (const terms of cases) {
      const lists: Required<Pick<ParsedQuery, "include" | "exclude">> = {
        include: {},
        exclude: {},
      };
      let searchQuery = "";
      for (const [id, value, exclude] of terms) {
        (lists[exclude ? "exclude" : "include"][id] ??= []).push(value);
        searchQuery = withTerm(searchQuery, id, value, exclude);
      }
      const keys = async (filters: QueryFilters) =>
        (await fetchRecords(filters, 100, -1, query)).map(({ key }) => key);
      expect([searchQuery, await keys({ searchQuery })]).toEqual([searchQuery, await keys(lists)]);
    }
  });

  it("matches words in values, not key names, and says where each row matched", async () => {
    connection.query(`INSERT INTO logs (EventID, Raw) VALUES
      (9, '{"Event_attributes":{"xmlns":"http://schemas.microsoft.com/win/2004/08/events/event"},"Event":{"System":{"EventID":9},"EventData":{"TargetUserName":"FSIR","CommandLine":"\\"C:\\\\a.exe\\" fsir"}}}')`);
    expect(await count("event")).toBe(0);
    expect(await count("system")).toBe(0);
    expect(await count("targetusername")).toBe(0);
    expect(await count("4624")).toBe(2);
    expect(await count("fsir")).toBe(1);
    expect(await count("a.exe")).toBe(1);
    expect(await count("-fsir")).toBe(3);
    expect(await matched("fsir")).toEqual(["TargetUserName: FSIR"]);
    expect(await matched("a.exe")).toEqual(['CommandLine: \\"C:\\a.exe\\" fsir']);
    expect(await matched("4625")).toEqual(["EventID: 4625"]);
    expect(await matched("event_id:9")).toEqual([undefined]);
  });

  it("keeps NULL rows when excluding values and leaves excluded values out of facets", async () => {
    expect(await countRecords({ exclude: { level: ["4"] } }, query)).toBe(2);
    expect(await countRecords({ exclude: { level: [""] } }, query)).toBe(3);
    expect(await countRecords({ exclude: { level: ["", "2"] } }, query)).toBe(2);
    expect(await countRecords({ exclude: { provider: ["Other"] } }, query)).toBe(1);
    expect(
      await getColumnFacetCounts("provider", { exclude: { provider: ["Other"] } }, 20, query),
    ).toEqual([{ v: "O'Brien", c: 1 }]);
  });

  it("returns typed facet counts and complete histogram boundaries", async () => {
    expect(await getColumnFacetCounts("eventId", {}, 20, query)).toEqual([
      { v: "4624", c: 2 },
      { v: "4625", c: 1 },
    ]);
    const buckets = await getTimeHistogram(
      { timeRange: { start: new Date(0), end: new Date(1) } },
      query,
    );
    expect(buckets[0].start).toBe(Date.parse("2025-01-01T00:00:00Z"));
    expect(buckets.at(-1)?.end).toBe(Date.parse("2025-01-02T00:00:00Z") + 1);
    expect(buckets.reduce((sum, bucket) => sum + bucket.count, 0)).toBe(2);
  });

  it("stacks histogram buckets by level group without losing events", async () => {
    connection.query(`DELETE FROM logs;
      INSERT INTO logs (EventID, Level, TimeCreated, Raw)
      SELECT i, [1, 2, 3, 4, 0, 5, NULL][i % 7 + 1], TIMESTAMP '2025-01-01' + INTERVAL (i * 7) MINUTE, '{}'
      FROM range(500) AS t(i)`);
    const buckets = await getTimeHistogram({}, query);
    expect(buckets.length).toBeGreaterThan(1);
    for (const bucket of buckets)
      expect(bucket.error + bucket.warning + bucket.information).toBe(bucket.count);
    const sum = (key: "count" | "error" | "warning" | "information") =>
      buckets.reduce((total, bucket) => total + bucket[key], 0);
    expect([sum("count"), sum("error"), sum("warning"), sum("information")]).toEqual([
      500, 144, 72, 284,
    ]);
  });

  it("groups null and empty facets before limiting and keeps self-filtered alternatives visible", async () => {
    connection.query(`DELETE FROM logs;
      INSERT INTO logs (EventID, Level, Provider, Channel, UserID, Raw) VALUES
        (1, NULL, NULL, 'Security', NULL, '{"Event":{"System":{}}}'),
        (2, 0, '', 'Security', '', '{"Event":{"System":{}}}'),
        (3, 4, 'Named', 'Security', 'Alice', '{"Event":{"System":{}}}')`);
    expect(await getColumnFacetCounts("user", {}, 1, query)).toEqual([{ v: "", c: 2 }]);
    expect(
      await getColumnFacetCounts("provider", { include: { provider: ["Named", ""] } }, 20, query),
    ).toEqual([
      { v: "", c: 2 },
      { v: "Named", c: 1 },
    ]);
    expect(
      await getColumnFacetCounts("level", { include: { level: ["0", ""] } }, 20, query),
    ).toEqual([
      { v: "", c: 1 },
      { v: "0", c: 1 },
      { v: "4", c: 1 },
    ]);
    expect(await countRecords({ include: { user: [""] } }, query)).toBe(2);
    // Excluding an empty value (from the details pane) keeps events without the field.
    expect(await countRecords({ exclude: { user: [""] } }, query)).toBe(2);
    expect(await countRecords({ include: { provider: [""] } }, query)).toBe(2);
    expect(await countRecords({ include: { provider: ["Named", ""] } }, query)).toBe(3);
    expect(await countRecords({ include: { level: [""] } }, query)).toBe(1);
    expect(await countRecords({ include: { level: ["0", ""] } }, query)).toBe(2);
    expect(
      await getColumnFacetCounts(
        "provider",
        { include: { provider: ["Named"], eventId: ["3"] } },
        20,
        query,
      ),
    ).toEqual([{ v: "Named", c: 1 }]);
  });

  it("stores real parser batches positionally as typed columns that match their JSON", async () => {
    // Needs the wasm-pack build, which CI runs before these checks (as tsc -b does).
    initSync({ module: readFileSync(new URL("../../wasm/evtx_wasm_bg.wasm", import.meta.url)) });
    const parser = new EvtxWasmParser(
      readFileSync(new URL("../../../../../samples/Security_short_selected.evtx", import.meta.url)),
    );
    connection.query("DELETE FROM logs");
    for (let index = 0; index < parser.total_chunks; index++) {
      const chunk = parser.chunk_arrow_ipc(index);
      connection.insertArrowFromIPCStream(chunk.ipc, { name: "logs", create: false });
      // A table created from the batch itself pins the Arrow types to LOGS_TABLE_SQL.
      if (!index) connection.insertArrowFromIPCStream(chunk.ipc, { name: "arrow", create: true });
      chunk.free();
    }
    parser.free();
    const columnTypes = (table: string) =>
      connection
        .query(`DESCRIBE ${table}`)
        .toArray()
        .map((column) => `${column.column_name} ${column.column_type}`);
    expect(columnTypes("arrow")).toEqual(columnTypes("logs"));
    connection.query("DROP TABLE arrow");
    const system = ["time", "task", "eventId", "level", "provider", "computer", "keywords"];
    const rows = await fetchTabular(
      system.map((id) => ({ id, header: id })),
      {},
      512,
      0,
      { id: "time", desc: false },
      query,
    );
    expect(rows).toHaveLength(7);
    for (const row of rows) {
      const record = await fetchRecordByKey(row.rowKey, query);
      const json = record?.Event.System;
      expect(row).toMatchObject({
        time: json?.TimeCreated_attributes?.SystemTime,
        task: json?.Task,
        eventId: json?.EventID,
        level: json?.Level,
        provider: json?.Provider_attributes?.Name,
        computer: json?.Computer,
        keywords: json?.Keywords,
      });
      for (const id of ["time", "task"]) {
        const include = { [id]: [String(row[id])] };
        expect(await countRecords({ include }, query)).toBeGreaterThan(0);
      }
    }
    const times = rows.map((row) => String(row.time));
    expect(times).toEqual(times.toSorted());
  });

  it("buckets time facets by calendar unit in the chosen zone, across DST", async () => {
    const saved = process.env.TZ;
    process.env.TZ = "America/New_York";
    try {
      const facet = (zone: "utc" | "local", filters = {}) =>
        getColumnFacetCounts("time", filters, 20, query, zone);
      const labels = async (zone: "utc" | "local") =>
        (await facet(zone)).map(({ v, c }) => [formatTimeBucket(v, zone), c]);
      // Hourly across the fall-back weekend: local Nov 6 has 25 hours.
      connection.query(`DELETE FROM logs;
        INSERT INTO logs (Level, TimeCreated, Raw)
        SELECT i % 5, TIMESTAMP '2016-11-04 04:00' + INTERVAL (i) HOUR, '{}' FROM range(96) AS t(i)`);
      expect(await labels("local")).toEqual([
        ["11/04/2016", 24],
        ["11/05/2016", 24],
        ["11/06/2016", 25],
        ["11/07/2016", 23],
      ]);
      expect(await labels("utc")).toEqual([
        ["11/04/2016", 20],
        ["11/05/2016", 24],
        ["11/06/2016", 24],
        ["11/07/2016", 24],
        ["11/08/2016", 4],
      ]);
      // Other filters narrow it; its own time range does not, and a bucket's range counts match it.
      const levelFour = { include: { level: ["4"] } };
      const buckets = await facet("local", {
        ...levelFour,
        timeRange: { start: new Date(0), end: new Date(1) },
      });
      expect(buckets.reduce((sum, { c }) => sum + c, 0)).toBe(await countRecords(levelFour, query));
      for (const { v, c } of buckets)
        expect(await countRecords({ ...levelFour, timeRange: parseTimeRangeKey(v) }, query)).toBe(
          c,
        );

      // Two days get hours, in each zone's own clock; nine months get months.
      connection.query(`DELETE FROM logs;
        INSERT INTO logs (TimeCreated, Raw) VALUES ('2016-10-06 04:30', '{}'), ('2016-10-08 03:00', '{}')`);
      expect(await labels("utc")).toEqual([
        ["10/06/2016, 04:00", 1],
        ["10/08/2016, 03:00", 1],
      ]);
      expect(await labels("local")).toEqual([
        ["10/06/2016, 00:00", 1],
        ["10/07/2016, 23:00", 1],
      ]);
      connection.query(`INSERT INTO logs (TimeCreated, Raw) VALUES ('2017-07-01 12:00', '{}')`);
      expect(await labels("local")).toEqual([
        ["Oct 2016", 2],
        ["Jul 2017", 1],
      ]);
    } finally {
      process.env.TZ = saved;
    }
  });

  it("propagates corrupt stored events and SQL errors rather than dropping evidence", async () => {
    connection.query("UPDATE logs SET Raw = 'not json' WHERE rowid = 0");
    await expect(fetchRecordByKey("0", query)).rejects.toThrow(SyntaxError);
    await expect(fetchRecords({}, 100, -1, query)).rejects.toThrow(SyntaxError);
    connection.query("DROP TABLE logs");
    await expect(countRecords({}, query)).rejects.toThrow(/logs/);
  });
});
