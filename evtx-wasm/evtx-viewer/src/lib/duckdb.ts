/* eslint-disable @typescript-eslint/no-explicit-any */
import * as duckdb from "@duckdb/duckdb-wasm";
import type { EvtxRecord, FilterOptions, BucketCounts } from "./types";

// ------------- File-session tracking -------------
// Each time we load a new log file we bump this counter.  Background inserts
// from previous sessions check the value and bail out early to avoid mixing
// data from multiple files.
let activeSessionId = 0;

export function beginNewSession(): number {
  return ++activeSessionId;
}

function isStale(sessionAtCall: number): boolean {
  return sessionAtCall !== activeSessionId;
}

// Keep a singleton instance so multiple components share the same DB
let db: duckdb.AsyncDuckDB | null = null;
let initPromise: Promise<any> | null = null;
let conn: any = null;

/**
 * Initialise DuckDB-WASM.  Call once on application startup.
 */
export async function initDuckDB(): Promise<any> {
  if (conn) return conn;
  if (initPromise) return initPromise;

  initPromise = (async () => {
    // Select the best JSDelivr bundle for the current browser
    const JSDELIVR_BUNDLES = duckdb.getJsDelivrBundles();
    const bundle = await duckdb.selectBundle(JSDELIVR_BUNDLES);

    // Create a same-origin wrapper worker to bypass cross-origin script restrictions.
    const workerBlobUrl = URL.createObjectURL(
      new Blob([`importScripts("${bundle.mainWorker}");`], {
        type: "text/javascript",
      })
    );

    const worker = new Worker(workerBlobUrl); // classic worker

    const logger = new duckdb.ConsoleLogger();
    db = new duckdb.AsyncDuckDB(logger, worker);
    await db.instantiate(bundle.mainModule, bundle.pthreadWorker);

    URL.revokeObjectURL(workerBlobUrl);

    conn = await db.connect();

    // Make sure a table exists – schema will be created automatically on first insert
    await conn.query(
      `CREATE TABLE IF NOT EXISTS logs (
      EventID INTEGER,
      Level   INTEGER,
      Provider TEXT,
      Channel  TEXT,
      Raw      TEXT
    );`
    );
    return conn;
  })();

  return initPromise;
}

// (Legacy ingestRecords and insertArrowBatch functions removed – Arrow IPC is now the sole ingestion path.)

export async function insertArrowIPC(
  buffer: Uint8Array | ArrayBuffer
): Promise<void> {
  const session = activeSessionId;
  const conn = await initDuckDB();
  if (isStale(session)) return;

  // Prefer new API name first (DuckDB ≥1.3 / wasm docs):
  // The `create` flag must be disabled here because we already create the
  // `logs` table (or ensure it exists) during `initDuckDB()`.  Leaving the
  // default (`create: true`) causes DuckDB to try to issue another
  //   CREATE TABLE logs (...)
  // for every batch which fails after the first batch with
  //   "Table with name \"logs\" already exists!".
  // See ArrowInsertOptions in duckdb-wasm docs.
  const insertOpts = { name: "logs", append: true, create: false } as const;
  conn.insertArrowFromIPCStream(buffer, insertOpts);
}

function escapeSqlString(str: string): string {
  return str.replace(/'/g, "''");
}

/** Build a SQL WHERE clause from current filters */
export function buildWhere(filters: FilterOptions): string {
  const clauses: string[] = [];

  if (filters.provider && filters.provider.length) {
    const list = filters.provider
      .map((p) => `'${escapeSqlString(p)}'`)
      .join(",");
    clauses.push(`Provider IN (${list})`);
  }

  if (filters.channel && filters.channel.length) {
    const list = filters.channel
      .map((c) => `'${escapeSqlString(c)}'`)
      .join(",");
    clauses.push(`Channel IN (${list})`);
  }

  if (filters.level && filters.level.length) {
    const list = filters.level.join(",");
    clauses.push(`Level IN (${list})`);
  }

  if (filters.eventId && filters.eventId.length) {
    const list = filters.eventId.join(",");
    clauses.push(`EventID IN (${list})`);
  }

  // New: EventData JSON field filters.  Each entry is AND-ed with the rest of
  // the WHERE clauses.  For a field “SubjectUserSid” with values ["S-1-5-18"],
  // we emit:
  //   json_extract_string(Raw, '$.Event.EventData.SubjectUserSid') IN ('S-1-5-18')
  if (filters.eventData) {
    for (const [field, values] of Object.entries(filters.eventData)) {
      if (!values || values.length === 0) continue;
      const valueList = values
        .map((v) => `'${escapeSqlString(String(v))}'`)
        .join(",");
      // Use DuckDB’s json_extract_string to pull the scalar value.
      const path = `$.Event.EventData.${field}`;
      clauses.push(`json_extract_string(Raw, '${path}') IN (${valueList})`);
    }
  }

  // EventData exclusion
  if (filters.eventDataExclude) {
    for (const [field, values] of Object.entries(filters.eventDataExclude)) {
      if (!values || values.length === 0) continue;
      const valueList = values
        .map((v) => `'${escapeSqlString(String(v))}'`)
        .join(",");
      const path = `$.Event.EventData.${field}`;
      clauses.push(`json_extract_string(Raw, '${path}') NOT IN (${valueList})`);
    }
  }

  if (filters.searchTerm && filters.searchTerm.trim() !== "") {
    const pattern = `%${escapeSqlString(filters.searchTerm.toLowerCase())}%`;
    clauses.push(
      `(lower(Provider) LIKE '${pattern}' OR lower(Channel) LIKE '${pattern}' OR cast(EventID as TEXT) LIKE '${pattern}')`
    );
  }

  // TODO: timeRange filter if needed
  return clauses.length ? clauses.join(" AND ") : "";
}

/**
 * Fetch aggregated facet counts given current filters.
 * Returns the counts for all Level/Provider/Channel/EventID values that still match.
 */
export async function getFacetCounts(
  filters: FilterOptions
): Promise<BucketCounts> {
  const c = await initDuckDB();

  const facetQueries: Record<keyof BucketCounts, string> = {
    level: "Level",
    provider: "Provider",
    channel: "Channel",
    event_id: "EventID",
  } as const;

  const result: BucketCounts = {
    level: {},
    provider: {},
    channel: {},
    event_id: {},
  };

  // Run queries sequentially – could be parallelised but fine for <100 facets
  for (const [bucketKey, col] of Object.entries(facetQueries) as [
    keyof BucketCounts,
    string
  ][]) {
    // For the EventID facet we want to ignore the current EventID filter so
    // that all IDs remain visible for multi-selection.
    let filtersForFacet: FilterOptions = filters;
    if (bucketKey === "event_id") {
      filtersForFacet = { ...filters, eventId: [] };
    }

    const whereFacet = buildWhere(filtersForFacet);
    const whereFacetSql = whereFacet ? `WHERE ${whereFacet}` : "";

    const res = await c.query(
      `SELECT ${col} as key, count(*) as cnt FROM logs ${whereFacetSql} GROUP BY ${col}`
    );

    // DuckDB may return bigint values for both the grouping key and the count
    // column.  Convert them to primitive JS numbers/strings so downstream code
    // can safely do arithmetic like `value + 1` without hitting the
    // "Cannot mix BigInt and other types" TypeError.
    for (const row of res.toArray() as { key: unknown; cnt: unknown }[]) {
      // Normalise the group key.  For numeric columns DuckDB can return a
      // BigInt – stringify first and then cast where appropriate so we keep
      // leading zeros etc. for text columns unchanged.
      const k = row.key === null ? "" : String(row.key);

      // The aggregate count is always numeric.  Convert BigInt → number; leave
      // plain numbers untouched.  Values here are expected to be < 2^53 which
      // is safe for JS Number.
      const cntNum: number =
        typeof row.cnt === "bigint" ? Number(row.cnt) : (row.cnt as number);

      (result[bucketKey] as Record<string, number>)[k] = cntNum;
    }
  }

  return result;
}

/**
 * Fetch paginated records matching the filters.
 */
export async function fetchRecords(
  filters: FilterOptions,
  limit = 100,
  offset = 0
): Promise<EvtxRecord[]> {
  const c = await initDuckDB();

  const where = buildWhere(filters);
  const whereSql = where ? `WHERE ${where}` : "";

  const res = await c.query(
    `SELECT Raw FROM logs ${whereSql} LIMIT ${limit} OFFSET ${offset}`
  );

  const out: EvtxRecord[] = [];
  for (const row of res.toArray() as { Raw: string }[]) {
    try {
      out.push(JSON.parse(row.Raw));
    } catch {
      /* ignore malformed */
    }
  }
  return out;
}

/** Remove all rows from the logs table – used when loading a new file. */
export async function clearLogs(): Promise<void> {
  const c = await initDuckDB();
  try {
    beginNewSession();
    await c.query("DELETE FROM logs");
  } catch (err) {
    // If the table somehow doesn’t exist yet just ignore.
    console.warn("DuckDB clearLogs failed", err);
  }
}

/** Count records matching current filters (fast aggregate). */
export async function countRecords(filters: FilterOptions): Promise<number> {
  const c = await initDuckDB();
  const where = buildWhere(filters);
  const whereSql = where ? `WHERE ${where}` : "";
  const res = await c.query(`SELECT count(*) as cnt FROM logs ${whereSql}`);
  const row = res.toArray()[0] as { cnt: number | bigint } | undefined;
  if (!row) return 0;
  return typeof row.cnt === "bigint" ? Number(row.cnt) : (row.cnt as number);
}

// (end of duckdb helpers)
