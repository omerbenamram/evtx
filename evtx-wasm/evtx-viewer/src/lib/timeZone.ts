import { useSyncExternalStore } from "react";

export type TimeZone = "local" | "utc";

const STORAGE_KEY = "time-zone";
const listeners = new Set<() => void>();
let current: TimeZone = readSaved();

function readSaved(): TimeZone {
  try {
    return localStorage.getItem(STORAGE_KEY) === "utc" ? "utc" : "local";
  } catch {
    return "local";
  }
}

/** View > Time zone. Every displayed event time follows it. */
export function setTimeZone(next: TimeZone): void {
  current = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch (cause) {
    console.warn("Time zone could not be saved.", cause);
  }
  for (const listener of listeners) listener();
}

/** Re-renders the caller when the time zone changes. */
export function useTimeZone(): TimeZone {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    () => current,
  );
}

export const timeZoneLabel = (zone: TimeZone = current) => (zone === "utc" ? "UTC" : "local time");

const formats = new Map<TimeZone, Intl.DateTimeFormat>();

/** Browser-locale date and time with milliseconds; "-" when missing, the input when unparseable. */
export function formatEventTime(
  value: string | number | Date | null | undefined,
  zone: TimeZone = current,
): string {
  if (value === null || value === undefined || value === "") return "-";
  const date = value instanceof Date ? value : new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  let format = formats.get(zone);
  if (!format) {
    format = new Intl.DateTimeFormat(undefined, {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      fractionalSecondDigits: 3,
      hourCycle: "h23",
      timeZone: zone === "utc" ? "UTC" : undefined,
    });
    formats.set(zone, format);
  }
  return format.format(date);
}

export const getTimeZone = (): TimeZone => current;

const SECOND = 1000;
const HOUR = 3600 * SECOND;
const DAY = 24 * HOUR;
// Finest first. Month and year sizes are averages, only good enough to pick a unit.
const SIZES = {
  second: SECOND,
  minute: 60 * SECOND,
  hour: HOUR,
  day: DAY,
  month: 30.436875 * DAY,
  year: 365.2425 * DAY,
};
type TimeUnit = keyof typeof SIZES;
// SAFETY: Object.entries of a literal yields exactly its keys, in declaration order.
const UNITS = Object.entries(SIZES) as [TimeUnit, number][];

function dateParts(time: number, utc: boolean): [number, number, number] {
  const date = new Date(time);
  return utc
    ? [date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()]
    : [date.getFullYear(), date.getMonth(), date.getDate()];
}
const midnight = (year: number, month: number, day: number, utc: boolean) =>
  utc ? Date.UTC(year, month, day) : new Date(year, month, day).getTime();

/** Start of the zone's calendar bucket holding `time`; days follow DST, so one may be 23 or 25 h. */
function bucketStart(time: number, unit: TimeUnit, utc: boolean): number {
  const [year, month, day] = dateParts(time, utc);
  if (unit === "year") return midnight(year, 0, 1, utc);
  if (unit === "month") return midnight(year, month, 1, utc);
  if (unit === "day") return midnight(year, month, day, utc);
  const size = SIZES[unit];
  const offset = utc ? 0 : -new Date(time).getTimezoneOffset() * 60 * SECOND;
  return time - ((((time + offset) % size) + size) % size);
}

function nextBucket(start: number, unit: TimeUnit, utc: boolean): number {
  const [year, month, day] = dateParts(start, utc);
  if (unit === "year") return midnight(year + 1, 0, 1, utc);
  if (unit === "month") return midnight(year, month + 1, 1, utc);
  if (unit === "day") return midnight(year, month, day + 1, utc);
  // ponytail: fixed steps assume whole-hour DST shifts (not Lord Howe's 30 minutes).
  return start + SIZES[unit];
}

/**
 * [start, next start) epoch-ms buckets covering lo..hi in the zone, at the finest unit that
 * keeps them to about `max`: a 2-day log gets hours, a 9-month log months.
 */
export function timeBuckets(lo: number, hi: number, zone: TimeZone, max = 60): [number, number][] {
  const unit = UNITS.find(([, size]) => (hi - lo) / size < max)?.[0] ?? "year";
  const utc = zone === "utc";
  const buckets: [number, number][] = [];
  for (let start = bucketStart(lo, unit, utc); start <= hi;) {
    const end = nextBucket(start, unit, utc);
    buckets.push([start, end]);
    start = end;
  }
  return buckets;
}

/** The facet value for a time range: `start/end` in epoch ms. */
export const timeRangeKey = ({ start, end }: { start: Date; end: Date }) =>
  `${start.getTime()}/${end.getTime()}`;

export function parseTimeRangeKey(value: string): { start: Date; end: Date } | undefined {
  const match = /^(-?\d+)\/(-?\d+)$/.exec(value);
  return match ? { start: new Date(Number(match[1])), end: new Date(Number(match[2])) } : undefined;
}

const bucketFormats = new Map<string, Intl.DateTimeFormat>();

/** "Oct 2016", "10/06/2016" or "10/06/2016, 04:00", by the bucket's length. */
export function formatTimeBucket(value: string, zone: TimeZone = current): string {
  const range = parseTimeRangeKey(value);
  if (!range) return value;
  const length = range.end.getTime() - range.start.getTime();
  // 0.9 lets 28-day months and 23-hour DST days still read as their unit.
  const unit = UNITS.findLast(([, size]) => length >= size * 0.9)?.[0] ?? "second";
  let format = bucketFormats.get(`${zone} ${unit}`);
  if (!format) {
    const clock = unit === "hour" || unit === "minute" || unit === "second";
    format = new Intl.DateTimeFormat(undefined, {
      year: "numeric",
      month: unit === "year" ? undefined : unit === "month" ? "short" : "2-digit",
      day: unit === "year" || unit === "month" ? undefined : "2-digit",
      hour: clock ? "2-digit" : undefined,
      minute: clock ? "2-digit" : undefined,
      second: unit === "second" ? "2-digit" : undefined,
      hourCycle: "h23",
      timeZone: zone === "utc" ? "UTC" : undefined,
    });
    bucketFormats.set(`${zone} ${unit}`, format);
  }
  return format.format(range.start);
}
