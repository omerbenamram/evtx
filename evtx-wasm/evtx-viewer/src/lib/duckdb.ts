import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { z } from "zod";
import {
  parseEvtxRecord,
  tabularRowSchema,
  type TableColumn,
  type TabularRow,
  type EvtxRecord,
  type FilterOptions,
} from "./types";
import { parseSearchQuery } from "./searchQuery";
import { columnSql } from "./columnSql";
import type { RowSort } from "./duckDbDataSource";

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
function rawText(term: string): string {
  return `contains(lower(Raw), '${escapeSqlString(JSON.stringify(term.toLowerCase()).slice(1, -1))}')`;
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

export function buildWhere(filters: FilterOptions): string {
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

export function whereClause(filters: FilterOptions): string {
  const where = buildWhere(filters);
  return where ? `WHERE ${where}` : "";
}

/** Matching records in file order after row key `afterKey`; keyset pages stay stable and linear. */
export async function fetchRecords(
  filters: FilterOptions,
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

export async function countRecords(filters: FilterOptions, query = queryLogs): Promise<number> {
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

export async function fetchTabular(
  columns: TableColumn[],
  filters: FilterOptions,
  limit = 512,
  offset = 0,
  sort: RowSort | null = null,
  query = queryLogs,
): Promise<TabularRow[]> {
  const fields = columns.map((col) => `${resolveColumn(col.id).sql} AS ${quoteIdentifier(col.id)}`);
  fields.push('CAST(rid AS VARCHAR) AS "rowKey"');
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
  filters: FilterOptions,
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

export async function getColumnFacetCounts(
  id: string,
  filters: FilterOptions,
  limit = 250,
  query = queryLogs,
) {
  const { sql, typed } = resolveColumn(id);
  // Leave out this column's own selections so users can multi-select; excluded values stay hidden.
  const whereSql = whereClause({ ...filters, include: { ...filters.include, [id]: [] } });
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
