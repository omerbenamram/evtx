import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { z } from "zod";
import {
  parseEvtxRecord,
  tabularRowSchema,
  type TableColumn,
  type TabularRow,
  type EvtxRecord,
  type FilterOptions,
  type ParsedQuery,
} from "./types";
import { parseSearchQuery, searchWords } from "./searchQuery";
import { columnSql } from "./columnSql";
import type { RowSort } from "./duckDbDataSource";
import { getTimeZone, timeBuckets, type TimeZone } from "./timeZone";

/** Bumped per cleared table so readers of an older log can tell their rows are gone. */
let activeSessionId = 0;

export function beginNewSession(): number {
  return ++activeSessionId;
}

export function getSessionId(): number {
  return activeSessionId;
}

/** Arrow IPC inserts are positional: keep in step with the wasm record batch. */
export const LOGS_TABLE_SQL = `CREATE TABLE IF NOT EXISTS logs (
  EventID INTEGER, Level INTEGER, Provider TEXT, Channel TEXT, TimeCreated TIMESTAMP,
  Computer TEXT, UserID TEXT, Task INTEGER, Opcode INTEGER, Keywords TEXT, Raw TEXT
)`;

let initPromise: Promise<AsyncDuckDBConnection> | undefined;

export function initDuckDB(): Promise<AsyncDuckDBConnection> {
  return (initPromise ??= openDuckDB().catch((cause: unknown) => {
    initPromise = undefined;
    throw cause;
  }));
}

async function openDuckDB(): Promise<AsyncDuckDBConnection> {
  const duckdb = await import("@duckdb/duckdb-wasm");
  const bundle = await duckdb.selectBundle(duckdb.getJsDelivrBundles());
  const workerUrl = URL.createObjectURL(
    new Blob([`importScripts(${JSON.stringify(bundle.mainWorker)});`], {
      type: "text/javascript",
    }),
  );
  let worker: Worker | undefined;
  try {
    worker = new Worker(workerUrl);
    const database = new duckdb.AsyncDuckDB(new duckdb.ConsoleLogger(), worker);
    await database.instantiate(bundle.mainModule, bundle.pthreadWorker);
    const connection = await database.connect();
    await connection.query(LOGS_TABLE_SQL);
    return connection;
  } catch (error) {
    worker?.terminate();
    throw error;
  } finally {
    URL.revokeObjectURL(workerUrl);
  }
}

export const queryLogs = async (sql: string) => (await initDuckDB()).query(sql);

const countSchema = z
  .union([z.number(), z.bigint()])
  .transform(Number)
  .pipe(z.number().int().min(0).max(Number.MAX_SAFE_INTEGER));
const countRowSchema = z.object({ cnt: countSchema });
const rawRowSchema = z.object({ Raw: z.string() });
const keyedRawRowSchema = rawRowSchema.extend({ key: countSchema });
const positionRowSchema = z.object({ position: countSchema });
const facetRowSchema = z.object({ v: z.string(), c: countSchema });
const epochMs = z.union([z.number(), z.bigint()]).transform(Number).nullable();
const timeBoundsSchema = z.object({ lo: epochMs, hi: epochMs });

/** No stale-session check: loadFile awaits the previous import before the next clearLogs. */
export async function insertArrowIPC(buffer: Uint8Array): Promise<void> {
  await (await initDuckDB()).insertArrowFromIPCStream(buffer, { name: "logs", create: false });
}

function escapeSqlString(str: string): string {
  return str.replace(/'/g, "''");
}

function resolveColumn(id: string) {
  const column = columnSql(id);
  if (!column) throw new Error(`Unknown column: ${id}`);
  return column;
}

// Raw is serialized JSON, so match the term as JSON would write it (C:\x is stored as C:\\x).
const jsonText = (term: string) => JSON.stringify(term.toLowerCase()).slice(1, -1);
const regexText = (text: string) => text.replace(/[\\^$.|?*+()[\]{}]/g, "\\$&");
// Every event carries this namespace; its "…/events/event" must not match the word "event".
const XMLNS =
  '"Event_attributes":{"xmlns":"http://schemas.microsoft.com/win/2004/08/events/event"}';
const STRING_BODY = String.raw`(?:[^"\\]|\\.)*`;

/** A string value (a string not followed by ":") or a number that contains the word. */
function valuePattern(term: string): string {
  const word = regexText(jsonText(term));
  const text = String.raw`"[^"]*${word}${STRING_BODY}"[,}\]]`;
  return /^[\d.]+$/.test(term) ? String.raw`${text}|[:,\[]-?[\d.]*${word}` : text;
}

/** Words match values, not key names; `contains` first skips most rows cheaply. */
function rawText(term: string): string {
  const lower = `replace(lower(Raw), '${escapeSqlString(XMLNS.toLowerCase())}', '')`;
  return `(contains(lower(Raw), '${escapeSqlString(jsonText(term))}') AND regexp_matches(${lower}, '${escapeSqlString(valuePattern(term))}'))`;
}

/** "Field: value" of the first value that contains `term`; an array item has no field name. */
export function matchedInSql(term: string): string {
  const word = regexText(jsonText(term));
  const pattern = String.raw`"([^"\\]*)":(?:"(${STRING_BODY}${word}${STRING_BODY})"|(-?[\d.]*${word}[\d.e+-]*))`;
  const raw = `replace(Raw, '${escapeSqlString(XMLNS)}', '')`;
  const group = (n: number) => `regexp_extract(${raw}, '${escapeSqlString(pattern)}', ${n}, 'i')`;
  // ponytail: only \\ is unescaped; decode other JSON escapes if they show up in practice.
  return `nullif(concat_ws(': ', nullif(${group(1)}, ''), replace(${group(2)} || ${group(3)}, '\\\\', '\\')), '')`;
}

/** Included "" also selects NULL, like the facets' "(Not set)"; exclude keeps NULL rows. */
function valuesPredicate(id: string, values: string[], exclude: boolean): string {
  const { sql, typed = sql } = resolveColumn(id);
  const list = values.map((value) => `'${escapeSqlString(value)}'`).join(",");
  // Text literals convert to the column's type; '' cannot, so a list with it compares text.
  const unset = values.includes("");
  const key = unset ? `CAST(${sql} AS VARCHAR)` : typed;
  if (exclude) return `(${typed} IS NULL OR ${key} NOT IN (${list}))`;
  return `${unset ? `coalesce(${key}, '')` : key} IN (${list})`;
}

export type QueryFilters = FilterOptions & ParsedQuery;

export function buildWhere(filters: QueryFilters): string {
  const clauses: string[] = [];
  if (filters.searchQuery?.trim()) {
    const queryWhere = buildWhere(parseSearchQuery(filters.searchQuery));
    if (queryWhere) clauses.push(`(${queryWhere})`);
  }

  for (const [id, values] of Object.entries(filters.include ?? {}))
    if (values.length) clauses.push(valuesPredicate(id, values, false));
  for (const [id, values] of Object.entries(filters.exclude ?? {}))
    if (values.length) clauses.push(valuesPredicate(id, values, true));

  for (const term of filters.contains ?? []) clauses.push(rawText(term));
  for (const term of filters.notContains ?? []) clauses.push(`NOT ${rawText(term)}`);

  if (filters.timeRange) {
    const { start, end } = filters.timeRange;
    if (!Number.isFinite(start.getTime()) || !Number.isFinite(end.getTime()) || start > end) {
      throw new Error("Choose a valid start and end time.");
    }
    clauses.push(`TimeCreated >= epoch_ms(${start.getTime()})`);
    clauses.push(`TimeCreated < epoch_ms(${end.getTime()})`);
  }

  return clauses.join(" AND ");
}

export function whereClause(filters: QueryFilters): string {
  const where = buildWhere(filters);
  return where ? `WHERE ${where}` : "";
}

/** Matching records in file order after row key `afterKey`; keyset pages stay stable and linear. */
export async function fetchRecords(
  filters: QueryFilters,
  limit = 100,
  afterKey = -1,
  query = queryLogs,
): Promise<{ key: number; record: EvtxRecord }[]> {
  const where = buildWhere(filters);
  const res = await query(`SELECT rowid AS key, Raw FROM logs
    WHERE rowid > ${afterKey} ${where ? `AND (${where})` : ""} ORDER BY rowid LIMIT ${limit}`);
  return res.toArray().map((row) => {
    const { key, Raw } = keyedRawRowSchema.parse(row);
    return { key, record: parseEvtxRecord(Raw) };
  });
}

export async function clearLogs(): Promise<void> {
  beginNewSession();
  await queryLogs("DELETE FROM logs");
}

export async function countRecords(filters: QueryFilters, query = queryLogs): Promise<number> {
  const res = await query(`SELECT count(*) as cnt FROM logs ${whereClause(filters)}`);
  return countRowSchema.parse(res.toArray()[0]).cnt;
}

function quoteIdentifier(value: string): string {
  return `"${value.replace(/"/g, '""')}"`;
}

export function buildOrderBy(
  columns: TableColumn[],
  sort: RowSort | null,
  rowid = "rowid",
): string {
  if (!sort || !columns.some((col) => col.id === sort.id)) return `${rowid} ASC`;
  const { sql, typed = sql } = resolveColumn(sort.id);
  return `${typed} ${sort.desc ? "DESC" : "ASC"} NULLS LAST, ${rowid} ASC`;
}

/** The grid's "Matched in" column while the query has words; it is not a real column. */
export const MATCHED_COLUMN_ID = "matchedIn";

export async function fetchTabular(
  columns: TableColumn[],
  filters: QueryFilters,
  limit = 512,
  offset = 0,
  sort: RowSort | null = null,
  query = queryLogs,
): Promise<TabularRow[]> {
  const fields = columns.map((col) => `${resolveColumn(col.id).sql} AS ${quoteIdentifier(col.id)}`);
  fields.push('CAST(rid AS VARCHAR) AS "rowKey"');
  const [word] = searchWords(filters.searchQuery);
  if (word) fields.push(`${matchedInSql(word)} AS "${MATCHED_COLUMN_ID}"`);
  // Slice the page first so display expressions run on its rows only, not every match.
  const order = buildOrderBy(columns, sort, "rid");
  const res = await query(`SELECT ${fields.join(", ")} FROM (
    SELECT *, rowid AS rid FROM logs ${whereClause(filters)}
    ORDER BY ${order} LIMIT ${limit} OFFSET ${offset}
  ) ORDER BY ${order}`);
  return res.toArray().map((row) => tabularRowSchema.parse({ ...row }));
}

export async function fetchRecordByKey(key: string, query = queryLogs): Promise<EvtxRecord | null> {
  if (!/^\d+$/.test(key)) throw new Error("Invalid event key");
  const result = await query(`SELECT Raw FROM logs WHERE rowid = ${key}`);
  const row = result.toArray()[0];
  return row ? parseEvtxRecord(rawRowSchema.parse(row).Raw) : null;
}

export async function findRowIndex(
  key: string,
  filters: QueryFilters,
  columns: TableColumn[],
  sort: RowSort | null,
  query = queryLogs,
): Promise<number | null> {
  if (!/^\d+$/.test(key)) throw new Error("Invalid event key");
  const result = await query(`SELECT position FROM (
    SELECT rowid AS event_key, row_number() OVER (ORDER BY ${buildOrderBy(columns, sort)}) - 1 AS position
    FROM logs ${whereClause(filters)}
  ) WHERE event_key = ${key}`);
  const row = result.toArray()[0];
  return row ? positionRowSchema.parse(row).position : null;
}

/** Time counts by calendar bucket in `zone`, oldest first; each value is a `timeRangeKey`. */
async function getTimeFacetCounts(whereSql: string, zone: TimeZone, query: typeof queryLogs) {
  const bounds =
    await query(`SELECT epoch_ms(min(TimeCreated)) AS lo, epoch_ms(max(TimeCreated)) AS hi
    FROM logs ${whereSql}`);
  const { lo, hi } = timeBoundsSchema.parse(bounds.toArray()[0]);
  if (lo === null || hi === null) return [];
  // ponytail: events without a time get no bucket; they are rare in real logs.
  const buckets = timeBuckets(lo, hi, zone).map(([start, end]) => `(${start}, ${end})`);
  const res = await query(`SELECT concat(bucket_start, '/', bucket_end) AS v, count(*) AS c
    FROM logs JOIN (VALUES ${buckets.join(",")}) AS b(bucket_start, bucket_end)
      ON TimeCreated >= epoch_ms(bucket_start) AND TimeCreated < epoch_ms(bucket_end)
    ${whereSql} GROUP BY bucket_start, bucket_end ORDER BY bucket_start`);
  return res.toArray().map((row) => facetRowSchema.parse(row));
}

export async function getColumnFacetCounts(
  id: string,
  filters: QueryFilters,
  limit = 250,
  query = queryLogs,
  zone = getTimeZone(),
) {
  const { sql, typed } = resolveColumn(id);
  // Leave out this column's own selections so users can multi-select; excluded values stay hidden.
  const { searchQuery, ...rest } = filters;
  const parsed = { ...rest, ...parseSearchQuery(searchQuery ?? "") };
  const own = { ...parsed, include: { ...parsed.include, [id]: [] } };
  // Time's own selection also covers the time range, which its buckets set.
  if (id === "time")
    return getTimeFacetCounts(whereClause({ ...own, timeRange: undefined }), zone, query);
  const whereSql = whereClause(own);
  const value = `coalesce(CAST(${sql} AS VARCHAR), '')`;
  // A typed column groups and ranks before formatting; its ISO text sorts like the timestamp.
  const res = await query(
    typed
      ? `SELECT ${value} AS v, c FROM (
          SELECT ${typed}, count(*) AS c FROM logs ${whereSql}
          GROUP BY ${typed} ORDER BY c DESC, ${typed} NULLS FIRST LIMIT ${limit}
        ) ORDER BY c DESC, v`
      : `SELECT ${value} AS v, count(*) c FROM logs ${whereSql} GROUP BY v ORDER BY c DESC, v ASC LIMIT ${limit}`,
  );
  return res.toArray().map((row) => facetRowSchema.parse(row));
}
