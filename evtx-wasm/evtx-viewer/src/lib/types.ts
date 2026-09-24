import type { ReactNode } from "react";
import { z } from "zod";

export const evtxFileInfoSchema = z.object({
  fileName: z.string(),
  totalChunks: z.number().int().nonnegative(),
});
export type EvtxFileInfo = z.infer<typeof evtxFileInfoSchema>;

const providerSchema = z
  .object({ Name: z.string().optional(), Guid: z.string().optional() })
  .catchall(z.json());
const timeSchema = z.object({ SystemTime: z.string().optional() }).catchall(z.json());
const securitySchema = z.object({ UserID: z.string().optional() }).catchall(z.json());
const evtxSystemSchema = z
  .object({
    Provider_attributes: providerSchema.nullish(),
    // Forwarded (WEC) events carry numeric System values as strings.
    EventID: z.union([z.number(), z.string()]).nullish(),
    Level: z.union([z.number(), z.string()]).nullish(),
    TimeCreated_attributes: timeSchema.nullish(),
    Channel: z.string().nullish(),
    Computer: z.string().nullish(),
    Security_attributes: securitySchema.nullish(),
  })
  .catchall(z.json());
const evtxEventDataSchema = z.record(z.string(), z.json());
const evtxRecordSchema = z
  .object({
    Event: z
      .object({
        System: evtxSystemSchema,
        EventData: evtxEventDataSchema.nullish(),
        UserData: z.json().optional(),
        RenderingInfo: z.json().optional(),
      })
      .catchall(z.json()),
  })
  .catchall(z.json());
export type EvtxRecord = z.infer<typeof evtxRecordSchema>;
type EvtxEventData = z.infer<typeof evtxEventDataSchema>;

export function parseEvtxRecord(json: string): EvtxRecord {
  return evtxRecordSchema.parse(JSON.parse(json));
}

export function formatEventValue(value: z.infer<ReturnType<typeof z.json>> | undefined): string {
  if (value === undefined || value === null) return "-";
  const text = z.string().safeParse(value);
  return text.success ? text.data : JSON.stringify(value);
}

// Level 0 (LogAlways) reads "Information", as in Event Viewer.
const EVENT_LEVEL_NAMES = new Map([
  [0, "Information"],
  [1, "Critical"],
  [2, "Error"],
  [3, "Warning"],
  [4, "Information"],
  [5, "Verbose"],
]);
export function levelName(level: string | number | boolean | null | undefined): string {
  return level === null || level === undefined
    ? "Unknown"
    : (EVENT_LEVEL_NAMES.get(Number(level)) ?? String(level));
}

const unnamedDataSchema = z.object({ "#text": z.json() });
export function getEventDataFields(data: EvtxEventData): { name: string; value: string }[] {
  return Object.entries(data).map(([name, value]) => {
    // Unnamed <Data> items render as {"#text": value}; eventDataExpression reads the same path.
    const unnamed = name === "Data" ? unnamedDataSchema.safeParse(value) : undefined;
    return { name, value: formatEventValue(unnamed?.success ? unnamed.data["#text"] : value) };
  });
}

export const tabularRowSchema = z
  .object({ rowKey: z.string() })
  .catchall(z.union([z.string(), z.number(), z.boolean(), z.null(), z.bigint().transform(String)]));
export type TabularRow = z.infer<typeof tabularRowSchema>;

export interface TableColumn {
  hidden?: boolean;
  id: string;
  header: string;
  accessor?: (row: TabularRow) => ReactNode;
  /** Set by the user (resize, saved view); unset columns size to their content. */
  width?: number;
  align?: "right";
}

/**
 * Filter values are strings keyed by column id. An included "" also matches NULL, like the
 * facets' "(Not set)"; excluding a value always keeps rows where the column is NULL.
 */
export interface FilterOptions {
  searchQuery?: string;
  /** Case-insensitive substrings of the raw event JSON. */
  contains?: string[];
  notContains?: string[];
  timeRange?: { start: Date; end: Date };
  include?: Record<string, string[]>;
  exclude?: Record<string, string[]>;
}

export const errorMessage = (cause: unknown, fallback = String(cause)) =>
  cause instanceof Error ? cause.message : fallback;
