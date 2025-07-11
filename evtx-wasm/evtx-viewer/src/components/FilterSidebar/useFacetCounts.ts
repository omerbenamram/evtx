import { useEffect, useState } from "react";
import { getColumnFacetCounts } from "../../lib/duckdb";
import { EXCLUDE_FROM_FACETS } from "./facetUtils";
import {
  useColumnsState,
  useFiltersState,
  useIngestState,
} from "../../state/store";
import type { ColumnSpec } from "../../lib/types";

/**
 * Hook that derives facet value → count maps for the active columns
 * using DuckDB.  Counts recompute automatically whenever filters, columns or
 * ingest progress change (sourced from the global store).
 */
export function useFacetCounts(): Record<string, Map<string, number>> {
  const columns = useColumnsState();
  const filters = useFiltersState();
  const { progress: ingestProgress } = useIngestState();
  const [counts, setCounts] = useState<Record<string, Map<string, number>>>({});

  // Exclusion list comes from facetUtils so the set stays centralised.

  useEffect(() => {
    if (ingestProgress < 1) return;

    let cancelled = false;

    (async () => {
      const out: Record<string, Map<string, number>> = {};

      // Ensure Channel column spec exists for facet counts even if not visible.
      const specs: ColumnSpec[] = [...columns];
      if (!specs.some((c) => c.id === "channel")) {
        specs.push({ id: "channel", header: "Channel", sqlExpr: "Channel" });
      }

      const facetableSpecs = specs.filter(
        (spec) => !EXCLUDE_FROM_FACETS.has(spec.id)
      );

      await Promise.all(
        facetableSpecs.map(async (spec) => {
          try {
            const res = await getColumnFacetCounts(spec, filters, 200);
            const m = new Map<string, number>();
            res.forEach(({ v, c }) => m.set(String(v), Number(c)));
            out[spec.id] = m;
          } catch (err) {
            console.warn(`facet counts failed for ${spec.id}`, err);
          }
        })
      );

      if (!cancelled) setCounts(out);
    })();

    return () => {
      cancelled = true;
    };
  }, [columns, filters, ingestProgress]);

  return counts;
}
