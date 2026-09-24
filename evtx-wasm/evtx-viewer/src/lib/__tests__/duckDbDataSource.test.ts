import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import {
  DuckDbDataSource,
  IMPORT_REFRESH_MS,
  MAX_CACHED_PAGES,
  PAGE_SIZE,
  type RowSort,
} from "../duckDbDataSource";
import { beginNewSession, getSessionId } from "../duckdb";
import type { FilterOptions } from "../types";
import { deferred, openTestDatabase } from "./database";

let connection: Awaited<ReturnType<typeof openTestDatabase>>;
const query = async (sql: string) => connection.query(sql);
const columns = [{ id: "eventId", header: "Event ID" }];
const row = (index: number) => ({ rowKey: String(index), eventId: index });

beforeAll(async () => {
  connection = await openTestDatabase();
});
afterAll(() => connection.close());
beforeEach(() => {
  beginNewSession();
  connection.query(`DROP TABLE IF EXISTS logs;
    CREATE TABLE logs AS SELECT i::INTEGER AS EventID, json_object('Event', json_object('System', json_object('EventID', i)))::VARCHAR AS Raw FROM range(10000) AS n(i)`);
});

describe("paged real event data", () => {
  it("shares filtered counts across sort variants of the same snapshot", async () => {
    let calls = 0;
    const countingQuery = async (sql: string) => {
      calls++;
      return connection.query(sql);
    };
    const source = new DuckDbDataSource(
      { searchQuery: "event_id:1 event_id:2 event_id:3" },
      columns,
      null,
      undefined,
      undefined,
      countingQuery,
    );
    const sorted = source.withSort({ id: "eventId", desc: true });
    await Promise.all([source.init(), sorted.init()]);
    expect(calls).toBe(1);
    expect(source.getTotalRecords()).toBe(3);
    expect(sorted.getTotalRecords()).toBe(3);
  });

  it("deduplicates page queries, resolves boundaries, and evicts the least recently used page", async () => {
    let calls = 0;
    const countingQuery = async (sql: string) => {
      calls++;
      return connection.query(sql);
    };
    const source = new DuckDbDataSource({}, columns, null, undefined, undefined, countingQuery);
    await Promise.all([source.getPage(0), source.getPage(0)]);
    expect(calls).toBe(1);
    expect(await source.getRow(PAGE_SIZE)).toEqual(row(PAGE_SIZE));
    for (let page = 2; page < MAX_CACHED_PAGES; page++) await source.getPage(page);
    await source.getPage(0);
    await source.getPage(MAX_CACHED_PAGES);
    expect(source.peekRow(0)).toEqual(row(0));
    expect(source.peekRow(PAGE_SIZE)).toBeUndefined();
    expect(source.peekRow(MAX_CACHED_PAGES * PAGE_SIZE)).toEqual(row(MAX_CACHED_PAGES * PAGE_SIZE));
  });

  it("ignores stale file responses and prevents late offscreen pages from displacing the viewport", async () => {
    const release = deferred();
    let calls = 0;
    const delayed = async (sql: string) => {
      const result = connection.query(sql);
      if (++calls === 1) await release.promise;
      return result;
    };
    const source = new DuckDbDataSource({}, columns, null, undefined, undefined, delayed);
    source.setVisiblePages(0, 0);
    const pending = source.getPage(0);
    source.setVisiblePages(8, 8);
    await source.getPage(8);
    release.resolve();
    await pending;
    expect(source.peekRow(0)).toBeUndefined();
    expect(source.peekRow(8 * PAGE_SIZE)).toEqual(row(8 * PAGE_SIZE));

    const staleRelease = deferred();
    const stale = new DuckDbDataSource(
      {},
      columns,
      null,
      undefined,
      undefined,
      async (sql: string) => {
        const result = connection.query(sql);
        await staleRelease.promise;
        return result;
      },
    );
    const loading = stale.getPage(0);
    beginNewSession();
    staleRelease.resolve();
    await expect(loading).rejects.toMatchObject({ name: "AbortError" });
    expect(stale.peekRow(0)).toBeUndefined();
  });

  it("keeps stable keys while sorting and recovers from real database query failures", async () => {
    const source = new DuckDbDataSource({}, columns, null, undefined, 10000, query);
    await source.init();
    expect(source.getTotalRecords()).toBe(10000);
    const sorted = source.withSort({ id: "eventId", desc: true });
    expect(sorted.queryKey).not.toBe(source.queryKey);
    expect(await sorted.getRow(0)).toEqual(row(9999));
    expect(await sorted.getRecord("9999")).toEqual({ Event: { System: { EventID: 9999 } } });
    expect(await sorted.indexOf("9999")).toBe(0);
    connection.query("ALTER TABLE logs RENAME TO held_logs");
    await expect(source.getPage(1)).rejects.toThrow(/logs/);
    connection.query("ALTER TABLE held_logs RENAME TO logs");
    expect(await source.getPage(1)).toHaveLength(PAGE_SIZE);
  });

  it("reuses complete unsorted pages after append but reloads partial tails", async () => {
    connection.query("DELETE FROM logs WHERE EventID >= 600");
    const previous = new DuckDbDataSource({}, columns, null, undefined, 600, query);
    await previous.getPage(0);
    expect(await previous.getPage(1)).toHaveLength(88);
    connection.query(
      "INSERT INTO logs SELECT i::INTEGER, json_object('Event', json_object('System', json_object('EventID', i)))::VARCHAR FROM range(600, 1200) AS n(i)",
    );
    const next = new DuckDbDataSource({}, columns, null, undefined, 1200, query);
    next.reusePages(previous);
    expect(next.peekRow(0)).toEqual(row(0));
    expect(next.peekRow(512)).toBeUndefined();
    await next.getPage(1);
    expect(next.peekRow(1023)?.eventId).toBe(1023);
    for (const incompatible of [
      previous.withSort({ id: "eventId", desc: true }),
      new DuckDbDataSource(
        { searchQuery: "event_id:42" },
        columns,
        null,
        undefined,
        undefined,
        query,
      ),
      new DuckDbDataSource(
        {},
        [...columns, { id: "level", header: "Level" }],
        null,
        undefined,
        undefined,
        query,
      ),
      new DuckDbDataSource({}, columns, null, getSessionId() + 1, undefined, query),
    ]) {
      incompatible.reusePages(previous);
      expect(incompatible.peekRow(0)).toBeUndefined();
    }
  });

  it("refreshes only unsorted, unfiltered views on every import tick", () => {
    const at = (loaded: number, filters: FilterOptions = {}, sort: RowSort | null = null) =>
      new DuckDbDataSource(filters, columns, null, undefined, loaded, query).withSort(sort);
    const sort = { id: "eventId", desc: true };
    const filtered = { searchQuery: "event_id:1" };
    const invalid = { timeRange: { start: new Date(2), end: new Date(1) } };
    const cleared = { searchQuery: " " };
    expect(at(200, cleared).replaces(at(100, cleared), true)).toBe(true);
    for (const [next, shown] of [
      [at(200, {}, sort), at(100, {}, sort)],
      [at(200, filtered), at(100, filtered)],
      [at(200, invalid), at(100, invalid)],
    ]) {
      expect(next.replaces(shown, true)).toBe(false);
      expect(next.replaces(shown, true, performance.now() + IMPORT_REFRESH_MS)).toBe(true);
      expect(next.replaces(shown, false)).toBe(true);
    }
    expect(at(200, filtered).replaces(at(100, {}, sort), true)).toBe(true);
    const resized = new DuckDbDataSource(
      {},
      [{ ...columns[0], width: 300 }],
      null,
      undefined,
      100,
      query,
    );
    expect(resized.replaces(at(100), false)).toBe(false);
  });

  it("accepts a late complete page across refresh while discarding its old partial tail", async () => {
    connection.query("DELETE FROM logs WHERE EventID >= 600");
    const release = deferred();
    const delayed = async (sql: string) => {
      const result = connection.query(sql);
      await release.promise;
      return result;
    };
    const previous = new DuckDbDataSource({}, columns, null, undefined, 600, delayed);
    const pending = [previous.getPage(0), previous.getPage(1)];
    const next = new DuckDbDataSource({}, columns, null, undefined, 1200, delayed);
    next.reusePages(previous);
    release.resolve();
    await Promise.all(pending);
    expect(next.peekRow(0)).toEqual(row(0));
    expect(next.peekRow(512)).toBeUndefined();
  });
});
