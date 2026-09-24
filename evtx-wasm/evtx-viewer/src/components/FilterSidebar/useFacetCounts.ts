import { useEffect, useState } from "react";
import { getColumnFacetCounts } from "../../lib/duckdb";
import { useColumnsState, useFiltersState, useEvtxMetaState } from "../../state/store";
import { buildFacetConfigs } from "./facetUtils";

export function useFacetCounts(): Record<string, Map<string, number>> {
  const columns = useColumnsState();
  const filters = useFiltersState();
  const { isLoading, fileInfo } = useEvtxMetaState();
  const [counts, setCounts] = useState<Record<string, Map<string, number>>>({});
  // Keyed on ids (XML names hold no newlines): a column resize must not re-run every scan.
  const facetIds = buildFacetConfigs(columns)
    .map(({ id }) => id)
    .join("\n");

  useEffect(() => {
    if (isLoading || !fileInfo) return;
    let cancelled = false;

    (async () => {
      const out: Record<string, Map<string, number>> = {};
      await Promise.all(
        facetIds.split("\n").map(async (id) => {
          try {
            const res = await getColumnFacetCounts(id, filters, 200);
            out[id] = new Map(res.map(({ v, c }) => [v, c]));
          } catch (err) {
            console.warn(`facet counts failed for ${id}`, err);
          }
        }),
      );

      if (!cancelled) setCounts(out);
    })();

    return () => {
      cancelled = true;
    };
  }, [facetIds, filters, isLoading, fileInfo]);

  return isLoading || !fileInfo ? {} : counts;
}
