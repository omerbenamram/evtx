import { z } from "zod";
import { queryLogs, whereClause } from "./duckdb";
import type { TimeZone } from "./timeZone";
import type { FilterOptions } from "./types";

const count = z.coerce.number().int().nonnegative();
const timeBucketSchema = z.object({
  start: z.coerce.number().int(),
  /** Exclusive upper boundary in epoch milliseconds. */
  end: z.coerce.number().int(),
  count,
  /** Critical and error (Level 1, 2). */
  error: count,
  /** Level 3. */
  warning: count,
  /** Everything else: information, LogAlways, verbose, unset. */
  information: count,
});

export type TimeBucket = z.infer<typeof timeBucketSchema>;

export async function getTimeHistogram(
  filters: FilterOptions,
  query = queryLogs,
): Promise<TimeBucket[]> {
  // Keep the full matching time extent visible while the user narrows its range.
  const result = await query(`
    WITH times AS (
      SELECT epoch_ms(TimeCreated) AS ts, Level
      FROM logs ${whereClause({ ...filters, timeRange: undefined })}
    ), bounds AS (
      SELECT min(ts) AS lo, max(ts) + 1 AS hi,
             least(48, max(ts) - min(ts) + 1) AS bins
      FROM times WHERE ts IS NOT NULL
    ), counts AS (
      SELECT least(bins - 1, floor((ts - lo) * bins / (hi - lo)))::INTEGER AS bucket,
             count(*) AS count,
             count(*) FILTER (WHERE Level IN (1, 2)) AS error,
             count(*) FILTER (WHERE Level = 3) AS warning,
             count(*) FILTER (WHERE Level IS NULL OR Level NOT IN (1, 2, 3)) AS information
      FROM times CROSS JOIN bounds WHERE ts IS NOT NULL GROUP BY bucket
    )
    SELECT ceil(lo + i * (hi - lo) / bins) AS start,
           ceil(lo + (i + 1) * (hi - lo) / bins) AS end,
           coalesce(counts.count, 0) AS count,
           coalesce(counts.error, 0) AS error,
           coalesce(counts.warning, 0) AS warning,
           coalesce(counts.information, 0) AS information
    FROM bounds CROSS JOIN range(48) AS sequence(i)
    LEFT JOIN counts ON counts.bucket = i
    WHERE lo IS NOT NULL AND i < bins ORDER BY i
  `);
  return result.toArray().map((row) => timeBucketSchema.parse(row));
}

export function rangeForBuckets(buckets: TimeBucket[], first: number, last: number) {
  const start = buckets[Math.min(first, last)];
  const end = buckets[Math.max(first, last)];
  return start && end ? { start: new Date(start.start), end: new Date(end.end) } : undefined;
}

const pad = (value: number, width = 2) => String(value).padStart(width, "0");

/** `2026-09-24 10:00:00.123` in the given zone, the format `parseTimeRange` reads back. */
export function formatTimeInput(date: Date, zone: TimeZone): string {
  const utc = zone === "utc";
  const parts = utc
    ? [date.getUTCFullYear(), date.getUTCMonth() + 1, date.getUTCDate()]
    : [date.getFullYear(), date.getMonth() + 1, date.getDate()];
  const time = utc
    ? [date.getUTCHours(), date.getUTCMinutes(), date.getUTCSeconds(), date.getUTCMilliseconds()]
    : [date.getHours(), date.getMinutes(), date.getSeconds(), date.getMilliseconds()];
  return `${pad(parts[0], 4)}-${pad(parts[1])}-${pad(parts[2])} ${pad(time[0])}:${pad(time[1])}:${pad(time[2])}.${pad(time[3], 3)}`;
}

// Invalid calendar dates and local times skipped by a DST change fail the round trip.
function parseTime(value: string, zone: TimeZone): Date {
  const parts = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3}))?)?$/.exec(
    value.trim(),
  );
  if (!parts) return new Date(NaN);
  const [year, month, day, hour, minute] = parts.slice(1, 6).map(Number);
  const second = Number(parts[6] ?? 0);
  const ms = Number((parts[7] ?? "").padEnd(3, "0"));
  const fields = [year, month - 1, day, hour, minute, second, ms] as const;
  const date = zone === "utc" ? new Date(Date.UTC(...fields)) : new Date(...fields);
  const back = formatTimeInput(date, zone);
  const expected = `${pad(year, 4)}-${pad(month)}-${pad(day)} ${pad(hour)}:${pad(minute)}:${pad(second)}.${pad(ms, 3)}`;
  return back === expected ? date : new Date(NaN);
}

export function parseTimeRange(start: string, end: string, zone: TimeZone) {
  const range = { start: parseTime(start, zone), end: parseTime(end, zone) };
  if (!Number.isFinite(range.start.getTime()) || !Number.isFinite(range.end.getTime())) {
    throw new Error("Use YYYY-MM-DD hh:mm[:ss.fff] for both times.");
  }
  if (range.start >= range.end) throw new Error("Start must be before end.");
  return range;
}
