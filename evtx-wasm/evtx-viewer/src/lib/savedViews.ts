import { z } from "zod";
import { buildEventDataColumn, getDefaultColumns } from "./columns";
import { columnSql, eventDataField } from "./columnSql";
import { parseSearchQuery } from "./searchQuery";
import type { TableColumn } from "./types";
import { MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH } from "../state/columns/columnsSlice";

export const SAVED_VIEWS_KEY = "evtx-viewer-saved-searches-v1";
export type SavedView = z.infer<typeof savedViewSchema>;

// Values are always escaped SQL literals; keys must name a known column, and typed columns
// only take values DuckDB can convert, since one bad literal fails every query of the view.
const columnFiltersSchema = z
  .record(z.string().max(256), z.array(z.string().max(4096)).max(256))
  .refine((columns) => Object.keys(columns).length <= 256)
  .refine((columns) =>
    Object.entries(columns).every(([id, values]) => {
      const column = columnSql(id);
      return (
        column !== undefined &&
        values.every((value) => !value || (column.values?.test(value) ?? true))
      );
    }),
  );
const savedDateSchema = z.iso
  .datetime()
  .refine(
    (value) =>
      Number.isFinite(new Date(value).getTime()) && new Date(value).toISOString() === value,
  )
  .transform((value) => new Date(value));
const querySchema = z
  .string()
  .max(8192)
  .refine((query) => {
    try {
      parseSearchQuery(query);
      return true;
    } catch {
      return false;
    }
  });
const savedFiltersSchema = z.object({
  searchQuery: querySchema.optional(),
  searchTerm: z.string().max(8192).optional(),
  include: columnFiltersSchema.optional(),
  exclude: columnFiltersSchema.optional(),
  timeRange: z
    .object({ start: savedDateSchema, end: savedDateSchema })
    .refine(({ start, end }) => start < end)
    .optional(),
});
const savedViewSchema = z.object({
  name: z
    .string()
    .max(80)
    .refine((name) => name.trim().length > 0),
  filters: savedFiltersSchema,
  columns: z
    .array(
      z.object({
        id: z.string(),
        width: z.number().min(MIN_COLUMN_WIDTH).max(MAX_COLUMN_WIDTH).optional(),
      }),
    )
    .min(1)
    .max(128)
    .refine((columns) => new Set(columns.map(({ id }) => id)).size === columns.length),
});
const savedDocumentSchema = z.object({ version: z.literal(1), views: z.array(z.json()).max(50) });

export function restoreColumns(saved: SavedView["columns"]): TableColumn[] {
  const defaults = getDefaultColumns();
  return saved.flatMap(({ id, width }) => {
    const field = eventDataField(id);
    const column =
      defaults.find((item) => item.id === id) ??
      (field !== undefined && /^[\w .:-]{1,128}$/.test(field)
        ? buildEventDataColumn(field)
        : undefined);
    return column ? [{ ...column, width: width ?? column.width }] : [];
  });
}

/** Stored views choose trusted columns; persisted SQL and functions are discarded. */
export function readSavedViews(raw: string | null): SavedView[] {
  if (!raw || raw.length > 1_000_000) return [];
  let document;
  try {
    document = savedDocumentSchema.safeParse(JSON.parse(raw));
  } catch {
    return [];
  }
  if (!document.success) return [];
  const views: SavedView[] = [];
  for (const entry of document.data.views) {
    const parsed = savedViewSchema.safeParse(entry);
    if (!parsed.success) continue;
    const view = parsed.data;
    if (views.some(({ name }) => name === view.name)) continue;
    if (restoreColumns(view.columns).length !== view.columns.length) continue;
    views.push(view);
  }
  return views;
}

export function writeSavedViews(views: SavedView[]): string {
  const serialized = JSON.stringify({ version: 1, views });
  if (readSavedViews(serialized).length !== views.length)
    throw new Error("This search cannot be saved. Check its columns and filters.");
  return serialized;
}
