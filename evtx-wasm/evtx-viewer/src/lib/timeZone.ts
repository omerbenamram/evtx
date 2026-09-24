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
