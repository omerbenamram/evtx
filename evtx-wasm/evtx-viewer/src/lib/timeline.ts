import { z } from "zod";
import { queryLogs, whereClause } from "./duckdb";
import type { FilterOptions } from "./types";

const timeBucketSchema = z.object({
  start: z.coerce.number().int(),
  /** Exclusive upper boundary in epoch milliseconds. */
  end: z.coerce.number().int(),
  count: z.coerce.number().int().nonnegative(),
});

export type TimeBucket = z.infer<typeof timeBucketSchema>;

export async function getTimeHistogram(
  filters: FilterOptions,
  query = queryLogs,
): Promise<TimeBucket[]> {
  // Keep the full matching time extent visible while the user narrows its range.
  const result = await query(`
    WITH times AS (
      SELECT epoch_ms(TimeCreated) AS ts
      FROM logs ${whereClause({ ...filters, timeRange: undefined })}
    ), bounds AS (
      SELECT min(ts) AS lo, max(ts) + 1 AS hi,
             least(48, max(ts) - min(ts) + 1) AS bins
      FROM times WHERE ts IS NOT NULL
    ), counts AS (
      SELECT least(bins - 1, floor((ts - lo) * bins / (hi - lo)))::INTEGER AS bucket,
             count(*) AS count
      FROM times CROSS JOIN bounds WHERE ts IS NOT NULL GROUP BY bucket
    )
    SELECT ceil(lo + i * (hi - lo) / bins) AS start,
           ceil(lo + (i + 1) * (hi - lo) / bins) AS end,
           coalesce(counts.count, 0) AS count
    FROM bounds CROSS JOIN range(48) AS sequence(i)
    LEFT JOIN counts ON counts.bucket = i
    WHERE lo IS NOT NULL AND i < bins ORDER BY i
  `);
  return result.toArray().map((row) => timeBucketSchema.parse(row));
}

export const formatUtc = (date: Date) =>
  date
    .toISOString()
    .replace("T", " ")
    .replace(/(\.000)?Z$/, " UTC");

export function rangeForBuckets(buckets: TimeBucket[], first: number, last: number) {
  const start = buckets[Math.min(first, last)];
  const end = buckets[Math.max(first, last)];
  return start && end ? { start: new Date(start.start), end: new Date(end.end) } : undefined;
}

const parseUtcTime = (value: string) => {
  const parts = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2})(?::(\d{2})(?:\.(\d{1,3}))?)?$/.exec(value);
  if (!parts) return new Date(NaN);
  const canonical = `${parts[1]}:${parts[2] ?? "00"}.${(parts[3] ?? "").padEnd(3, "0")}Z`;
  const date = new Date(canonical);
  return Number.isFinite(date.getTime()) && date.toISOString() === canonical ? date : new Date(NaN);
};

export function parseUtcRange(start: string, end: string) {
  const range = { start: parseUtcTime(start), end: parseUtcTime(end) };
  if (!Number.isFinite(range.start.getTime()) || !Number.isFinite(range.end.getTime())) {
    throw new Error("Enter a valid start and end time in UTC.");
  }
  if (range.start >= range.end) throw new Error("Start time must be before end time.");
  return range;
}
