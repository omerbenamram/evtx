import type { ColumnSpec } from "../../lib/types";
import type { FacetConfig } from "./FacetSection";

// Reusable helper that converts epoch-ms or ISO strings into a readable
// "YYYY-MM-DD HH:MM" 24-hour local string.
export function formatTimeValue(raw: string | number): string {
  let d: Date | null = null;
  if (typeof raw === "number") d = new Date(raw);
  else if (typeof raw === "string") {
    const num = Number(raw);
    if (!Number.isNaN(num)) d = new Date(num);
    else {
      const parsed = new Date(raw);
      if (!Number.isNaN(parsed.getTime())) d = parsed;
    }
  }
  if (!d || Number.isNaN(d.getTime())) return String(raw);
  return d.toLocaleString(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

// Mapping of Windows Event Levels to descriptive labels
const LEVEL_NAME_MAP: Record<number, string> = {
  0: "LogAlways",
  1: "Critical",
  2: "Error",
  3: "Warning",
  4: "Information",
  5: "Verbose",
};

// Columns that should not be shown as facet buckets.  Currently only the
// Certain high-cardinality columns are excluded from equality-style faceting.
// (The timestamp column is now supported via adaptive time-bucket grouping.)
export const EXCLUDE_FROM_FACETS = new Set<string>([
  /* add ids here as needed */
]);

/**
 * Build the complete list of facet configurations given the current active
 * table columns.
 *
 * – Built-in facets are always present (level, provider, channel, eventId)
 * – Any additional column that is not one of the built-ins becomes a dynamic
 *   facet so users can filter on arbitrary extracted fields.
 */
export function buildFacetConfigs(columns: ColumnSpec[]): FacetConfig[] {
  const builtins: FacetConfig[] = [
    {
      id: "level",
      label: "Level",
      filterKey: "level",
      displayValue: (v) => LEVEL_NAME_MAP[v as number] || String(v),
    },
    {
      id: "time",
      label: "Date / Time",
      // Uses columnEquals on "time" so no simple filterKey
      displayValue: (v) => formatTimeValue(v),
    },
    {
      id: "provider",
      label: "Provider",
      filterKey: "provider",
      searchable: true,
    },
    {
      id: "channel",
      label: "Channel",
      filterKey: "channel",
      searchable: true,
    },
    {
      id: "eventId",
      label: "Event ID",
      filterKey: "eventId",
    },
  ];

  const dynamicCols: FacetConfig[] = columns
    .filter(
      (c) =>
        !builtins.some((b) => b.id === c.id) && !EXCLUDE_FROM_FACETS.has(c.id)
    )
    .map((c) => ({ id: c.id, label: c.header }));

  return [...builtins, ...dynamicCols];
}
