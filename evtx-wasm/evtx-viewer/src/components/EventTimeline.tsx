import { useEffect, useRef, useState } from "react";
import { styled, useTheme } from "styled-components";
import { useFilters } from "../hooks/useFilters";
import {
  formatUtc,
  getTimeHistogram,
  parseUtcRange,
  rangeForBuckets,
  type TimeBucket,
} from "../lib/timeline";
import { Button, Input } from "./Windows";
import { errorMessage } from "../lib/types";

const Timeline = styled.section`
  flex-shrink: 0;
  padding: 8px 12px;
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.medium};
  background: ${({ theme }) => theme.colors.background.secondary};
  font-size: ${({ theme }) => theme.fontSize.caption};
`;
const Controls = styled.form`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
`;
const DateInput = styled(Input)`
  width: 212px;
  font-variant-numeric: tabular-nums;
`;
const Graph = styled.fieldset`
  border: 0;
  padding: 0;
  min-width: 0;
  height: 56px;
  margin: 6px 0;
  touch-action: none;
  cursor: crosshair;
  svg {
    display: block;
    width: 100%;
    height: 100%;
  }
`;
const Extent = styled.div`
  display: flex;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 4px 12px;

  color: ${({ theme }) => theme.colors.text.secondary};
  font-variant-numeric: tabular-nums;
`;
const Notice = styled.p<{ $error?: boolean }>`
  padding: 4px 0;
  color: ${({ theme, $error }) => ($error ? theme.colors.status.error : theme.colors.text.secondary)};
`;
const utcInput = (date?: Date) => (date ? date.toISOString().slice(0, -1) : "");

export function EventTimeline({
  disabled = false,
  fileId,
}: {
  disabled?: boolean;
  fileId?: string | null;
}) {
  const { filters, updateFilters } = useFilters();
  const theme = useTheme();
  const [result, setResult] = useState<{ key: string; buckets: TimeBucket[]; error: string }>({
    key: "",
    buckets: [],
    error: "",
  });
  const [rangeError, setRangeError] = useState("");
  const [retry, setRetry] = useState(0);
  const rangeKey = `${fileId ?? ""}:${utcInput(filters.timeRange?.start)}/${utcInput(filters.timeRange?.end)}`;
  const appliedDraft = {
    key: rangeKey,
    start: utcInput(filters.timeRange?.start),
    end: utcInput(filters.timeRange?.end),
  };
  const [draft, setDraft] = useState(appliedDraft);
  if (draft.key !== rangeKey) setDraft(appliedDraft);
  const { start, end } = draft;
  const [selection, setSelection] = useState<[number, number] | null>(null);
  const [cursor, setCursor] = useState(0);
  const dragging = useRef(false);
  const anchor = useRef(0);
  // The histogram ignores timeRange, so narrowing the range must not refetch it.
  // Without timeRange the filters are plain JSON and survive the round trip.
  const histogramFilters = JSON.stringify({ ...filters, timeRange: undefined });
  const requestKey = JSON.stringify([histogramFilters, fileId, retry]);
  const loading = !disabled && result.key !== requestKey;
  const buckets = disabled || loading ? [] : result.buckets;
  const loadError = disabled || loading ? "" : result.error;

  useEffect(() => {
    if (disabled) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      getTimeHistogram(JSON.parse(histogramFilters))
        .then((next) => {
          if (!cancelled) {
            setResult({ key: requestKey, buckets: next, error: "" });
            setCursor(0);
            setSelection(null);
          }
        })
        .catch((cause: unknown) => {
          if (!cancelled)
            setResult({
              key: requestKey,
              buckets: [],
              error: `Could not load event times. ${errorMessage(cause)}`,
            });
        });
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [histogramFilters, requestKey, disabled]);

  const peak = Math.max(1, ...buckets.map((bucket) => bucket.count));
  const total = buckets.reduce((sum, bucket) => sum + bucket.count, 0);
  const activeRange = selection ? rangeForBuckets(buckets, ...selection) : filters.timeRange;

  function applyBuckets(first: number, last: number) {
    const range = rangeForBuckets(buckets, first, last);
    if (range) updateFilters((current) => ({ ...current, timeRange: range }));
  }
  function bucketAt(clientX: number, element: HTMLElement) {
    const box = element.getBoundingClientRect();
    return Math.max(
      0,
      Math.min(buckets.length - 1, Math.floor(((clientX - box.left) / box.width) * buckets.length)),
    );
  }

  return (
    <Timeline aria-label="Event timeline" aria-busy={loading}>
      <Controls
        onSubmit={(event) => {
          event.preventDefault();
          try {
            const timeRange = parseUtcRange(start, end);
            updateFilters((current) => ({ ...current, timeRange }));
            setRangeError("");
          } catch (error) {
            setRangeError(errorMessage(error));
          }
        }}
      >
        <strong>Timeline (UTC)</strong>
        <label htmlFor="timeline-start">
          From
          <DateInput
            id="timeline-start"
            aria-label="From (UTC)"
            $compact
            type="datetime-local"
            step="0.001"
            required
            disabled={disabled}
            value={start}
            onChange={(event) => {
              setDraft({ key: rangeKey, start: event.target.value, end });
              setRangeError("");
            }}
          />
        </label>
        <label htmlFor="timeline-end">
          Before
          <DateInput
            id="timeline-end"
            aria-label="Before (UTC), exclusive"
            $compact
            type="datetime-local"
            step="0.001"
            required
            disabled={disabled}
            value={end}
            onChange={(event) => {
              setDraft({ key: rangeKey, start, end: event.target.value });
              setRangeError("");
            }}
          />
        </label>
        <Button type="submit" size="small" disabled={disabled || !start || !end}>
          Apply range
        </Button>
        {filters.timeRange && (
          <Button
            type="button"
            size="small"
            variant="subtle"
            disabled={disabled}
            onClick={() => {
              setSelection(null);
              updateFilters((current) => ({ ...current, timeRange: undefined }));
            }}
          >
            All time
          </Button>
        )}
      </Controls>
      {rangeError && (
        <Notice $error role="alert">
          {rangeError}
        </Notice>
      )}
      {loadError ? (
        <Notice $error role="alert">
          {loadError}{" "}
          <Button size="small" onClick={() => setRetry((value) => value + 1)}>
            Retry
          </Button>
        </Notice>
      ) : buckets.length > 0 ? (
        <>
          <Graph
            aria-label="Select a time range in the histogram"
            aria-describedby="timeline-help"
            title="Drag to select. Arrow keys move; Shift extends; Enter applies."
            tabIndex={0}
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              event.preventDefault();
              event.currentTarget.focus();
              event.currentTarget.setPointerCapture(event.pointerId);
              const index = bucketAt(event.clientX, event.currentTarget);
              dragging.current = true;
              anchor.current = index;
              setCursor(index);
              setSelection([index, index]);
            }}
            onPointerMove={(event) => {
              if (!dragging.current) return;
              const index = bucketAt(event.clientX, event.currentTarget);
              setCursor(index);
              setSelection([anchor.current, index]);
            }}
            onPointerUp={(event) => {
              if (!dragging.current) return;
              dragging.current = false;
              const index = bucketAt(event.clientX, event.currentTarget);
              applyBuckets(anchor.current, index);
              setSelection(null);
            }}
            onPointerCancel={() => {
              dragging.current = false;
              setSelection(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                setSelection(null);
                return;
              }
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                applyBuckets(...(selection ?? [cursor, cursor]));
                setSelection(null);
                return;
              }
              if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
              event.preventDefault();
              const next =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? buckets.length - 1
                    : Math.max(
                        0,
                        Math.min(
                          buckets.length - 1,
                          cursor + (event.key === "ArrowRight" ? 1 : -1),
                        ),
                      );
              if (!event.shiftKey) anchor.current = next;
              setCursor(next);
              setSelection([anchor.current, next]);
            }}
          >
            <svg
              viewBox={`0 0 ${buckets.length * 12} 56`}
              preserveAspectRatio="none"
              aria-label={`${total.toLocaleString()} matching events across ${buckets.length} time intervals`}
            >
              {buckets.map((bucket, index) => {
                const selected =
                  activeRange &&
                  bucket.start < activeRange.end.getTime() &&
                  bucket.end > activeRange.start.getTime();
                const height = bucket.count ? Math.max(2, (bucket.count / peak) * 52) : 1;
                return (
                  <rect
                    key={bucket.start}
                    x={index * 12}
                    y={56 - height}
                    width={11}
                    height={height}
                    fill={selected ? theme.colors.accent.primary : theme.colors.text.secondary}
                    opacity={activeRange && !selected ? 0.35 : 1}
                  >
                    <title>{`${formatUtc(new Date(bucket.start))} to before ${formatUtc(new Date(bucket.end))}: ${bucket.count.toLocaleString()} events`}</title>
                  </rect>
                );
              })}
            </svg>
          </Graph>
          <Extent>
            <span>{formatUtc(new Date(buckets[0].start))}</span>
            <span>{total.toLocaleString()} matching events</span>
            <span>{formatUtc(new Date(buckets[buckets.length - 1].end))}</span>
          </Extent>
        </>
      ) : (
        <Notice as="output">
          {disabled
            ? "Timeline will be available when import finishes."
            : loading
              ? "Loading event times…"
              : "No events with a timestamp match these filters."}
        </Notice>
      )}
      <Notice id="timeline-help" hidden>
        Drag to select a range. Arrow keys move; Shift extends; Enter applies. End time is
        exclusive.
      </Notice>
      {selection && buckets.length > 0 && (
        <Notice as="output">{`${formatUtc(new Date(buckets[Math.min(...selection)].start))} to before ${formatUtc(new Date(buckets[Math.max(...selection)].end))}`}</Notice>
      )}
    </Timeline>
  );
}
