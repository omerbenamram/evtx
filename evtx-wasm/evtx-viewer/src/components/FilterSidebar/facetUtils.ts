import type { ColumnSpec } from "../../lib/types";
import type { FacetConfig } from "./FacetSection";

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
// timestamp column is excluded because the values are effectively unique and
// therefore not useful for equality-based faceting.
export const EXCLUDE_FROM_FACETS = new Set(["time"]);

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
