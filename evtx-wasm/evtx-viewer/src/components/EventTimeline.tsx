import { useEffect, useRef, useState } from "react";
import { styled, useTheme } from "styled-components";
import { useFilters } from "../hooks/useFilters";
import {
  formatTimeInput,
  getTimeHistogram,
  parseTimeRange,
  rangeForBuckets,
  type TimeBucket,
} from "../lib/timeline";
import { formatEventTime, timeZoneLabel, useTimeZone, type TimeZone } from "../lib/timeZone";
import { Button, Input, Popover } from "./Windows";
import { errorMessage } from "../lib/types";

const GRAPH_HEIGHT = 56;

const Timeline = styled.section`
  flex-shrink: 0;
  padding: 2px 12px 4px;
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
  background: ${({ theme }) => theme.colors.surface.pane};
`;
const Header = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  height: 28px;
  h2 {
    font-size: ${({ theme }) => theme.fontSize.body};
    font-weight: 600;
  }
`;
const RangeText = styled.span<{ $applied: boolean }>`
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: ${({ theme, $applied }) => ($applied ? theme.colors.text.primary : theme.colors.text.secondary)};
  font-variant-numeric: tabular-nums;
`;
const Body = styled.div`
  height: ${GRAPH_HEIGHT + 18}px;
`;
const Graph = styled.fieldset<{ $bins: number }>`
  display: grid;
  min-width: 0;
  margin: 0;
  padding: 0;
  border: 0;
  grid-template-columns: repeat(${({ $bins }) => $bins}, minmax(0, 1fr));
  column-gap: 1px;
  height: ${GRAPH_HEIGHT}px;
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
  touch-action: none;
  cursor: crosshair;
  &:focus-visible {
    outline-offset: 2px;
  }
`;
const Bar = styled.div`
  grid-row: 1;
  display: flex;
  flex-direction: column-reverse;
  overflow: hidden;
  &[data-hover] {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  &[data-hover] > * {
    filter: brightness(0.85);
  }
`;
const Selection = styled.div`
  grid-row: 1;
  pointer-events: none;
  background: ${({ theme }) => theme.colors.fill.selected};
  border-inline: 1px solid ${({ theme }) => theme.colors.accent.rest};
`;
const Axis = styled.div`
  position: relative;
  height: 17px;
  font-size: ${({ theme }) => theme.fontSize.secondary};
  color: ${({ theme }) => theme.colors.text.tertiary};
  font-variant-numeric: tabular-nums;
  span {
    position: absolute;
    top: 2px;
    white-space: nowrap;
  }
`;
const Notice = styled.p<{ $error?: boolean }>`
  display: flex;
  align-items: center;
  gap: 8px;
  height: 100%;
  color: ${({ theme, $error }) => ($error ? theme.colors.severity.error : theme.colors.text.secondary)};
`;
const Tip = styled(Popover)`
  padding: 5px 8px 6px;
  pointer-events: none;
  font-variant-numeric: tabular-nums;
  dl {
    display: grid;
    grid-template-columns: auto auto;
    gap: 0 12px;
    margin-top: 4px;
  }
  dt {
    display: flex;
    align-items: center;
    gap: 6px;
    color: ${({ theme }) => theme.colors.text.secondary};
  }
  dd {
    margin: 0;
    text-align: right;
  }
`;
const Swatch = styled.i<{ $color: string }>`
  width: 8px;
  height: 8px;
  border-radius: 2px;
  background: ${({ $color }) => $color};
`;
const RangeForm = styled.form`
  display: grid;
  gap: 8px;
  padding: 8px;
  width: 260px;
  h3 {
    font-size: ${({ theme }) => theme.fontSize.body};
    font-weight: 600;
  }
  label {
    margin-bottom: -4px;
    color: ${({ theme }) => theme.colors.text.secondary};
  }
  input {
    font-variant-numeric: tabular-nums;
  }
  p {
    color: ${({ theme }) => theme.colors.severity.error};
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
`;

// Severity stack, bottom to top. Information is neutral, per DESIGN.md.
const LEVELS = [
  { key: "information", label: "Information" },
  { key: "warning", label: "Warning" },
  { key: "error", label: "Error / critical" },
] as const;

/** Axis ticks read to the minute; the header and tooltip keep milliseconds. */
const shortTime = (value: number, zone: TimeZone) =>
  formatEventTime(value, zone).replace(/:\d{2}[.,]\d{3}$/, "");

function CustomRange({
  anchor,
  initial,
  applied,
  zone,
  onApply,
  onClose,
}: {
  anchor: HTMLElement;
  initial: { start: Date; end: Date };
  applied: boolean;
  zone: TimeZone;
  onApply: (range: { start: Date; end: Date } | undefined) => void;
  onClose: () => void;
}) {
  const [start, setStart] = useState(() => formatTimeInput(initial.start, zone));
  const [end, setEnd] = useState(() => formatTimeInput(initial.end, zone));
  const [error, setError] = useState("");
  const startInput = useRef<HTMLInputElement>(null);
  // Focus moves into the flyout on open; Popover returns it to the trigger on close.
  useEffect(() => startInput.current?.focus(), []);
  return (
    <Popover anchor={anchor} onClose={onClose}>
      <RangeForm
        aria-label="Custom range"
        onSubmit={(event) => {
          event.preventDefault();
          try {
            onApply(parseTimeRange(start, end, zone));
          } catch (cause) {
            setError(errorMessage(cause));
          }
        }}
      >
        <h3>Custom range, {timeZoneLabel(zone)}</h3>
        <label htmlFor="custom-range-start">Start</label>
        <Input
          id="custom-range-start"
          ref={startInput}
          value={start}
          placeholder="YYYY-MM-DD hh:mm:ss.fff"
          aria-invalid={Boolean(error)}
          aria-describedby={error ? "custom-range-error" : undefined}
          onChange={(event) => {
            setStart(event.target.value);
            setError("");
          }}
        />
        <label htmlFor="custom-range-end">End (excluded)</label>
        <Input
          id="custom-range-end"
          value={end}
          placeholder="YYYY-MM-DD hh:mm:ss.fff"
          aria-invalid={Boolean(error)}
          aria-describedby={error ? "custom-range-error" : undefined}
          onChange={(event) => {
            setEnd(event.target.value);
            setError("");
          }}
        />
        {error && (
          <p id="custom-range-error" role="alert">
            {error}
          </p>
        )}
        <footer>
          <Button disabled={!applied} onClick={() => onApply(undefined)}>
            Clear
          </Button>
          <Button type="submit" variant="accent">
            Apply
          </Button>
        </footer>
      </RangeForm>
    </Popover>
  );
}

export function EventTimeline({
  disabled = false,
  fileId,
}: {
  disabled?: boolean;
  fileId?: string | null;
}) {
  const { filters, updateFilters } = useFilters();
  const zone = useTimeZone();
  const theme = useTheme();
  const [result, setResult] = useState<{
    key: string;
    fileId?: string | null;
    buckets: TimeBucket[];
    error: string;
  }>({ key: "", buckets: [], error: "" });
  const [retry, setRetry] = useState(0);
  const [selection, setSelection] = useState<[number, number] | null>(null);
  const [cursor, setCursor] = useState(0);
  const [hover, setHover] = useState<{ index: number; bar: Element } | null>(null);
  const [rangeAnchor, setRangeAnchor] = useState<HTMLElement | null>(null);
  const dragging = useRef(false);
  const anchor = useRef(0);
  // The histogram ignores timeRange, so narrowing the range must not refetch it.
  // Without timeRange the filters are plain JSON and survive the round trip.
  const histogramFilters = JSON.stringify({ ...filters, timeRange: undefined });
  const requestKey = JSON.stringify([histogramFilters, fileId, retry]);
  const loading = !disabled && result.key !== requestKey;
  // Keep the previous bars on screen while the same file's histogram refreshes.
  const buckets = disabled || (loading && result.fileId !== fileId) ? [] : result.buckets;
  const loadError = disabled || loading ? "" : result.error;

  useEffect(() => {
    if (disabled) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      getTimeHistogram(JSON.parse(histogramFilters))
        .then((next) => {
          if (!cancelled) {
            setResult({ key: requestKey, fileId, buckets: next, error: "" });
            setCursor(0);
            setSelection(null);
            setHover(null);
          }
        })
        .catch((cause: unknown) => {
          if (!cancelled)
            setResult({
              key: requestKey,
              fileId,
              buckets: [],
              error: `Could not load event times. ${errorMessage(cause)}`,
            });
        });
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [histogramFilters, requestKey, disabled, fileId]);

  const peak = Math.max(1, ...buckets.map((bucket) => bucket.count));
  const total = buckets.reduce((sum, bucket) => sum + bucket.count, 0);
  const activeRange = selection ? rangeForBuckets(buckets, ...selection) : filters.timeRange;
  const extent = buckets.length
    ? { start: new Date(buckets[0].start), end: new Date(buckets[buckets.length - 1].end) }
    : undefined;
  const shown = activeRange ?? extent;
  const colors = {
    information: `color-mix(in srgb, ${theme.colors.text.tertiary} 45%, transparent)`,
    warning: theme.colors.severity.warning,
    error: theme.colors.severity.error,
  };
  // Bars overlapping the range, including a typed range that starts or ends mid-bar.
  const first = activeRange
    ? buckets.findIndex((bucket) => bucket.end > activeRange.start.getTime())
    : -1;
  const last = activeRange
    ? buckets.findLastIndex((bucket) => bucket.start < activeRange.end.getTime())
    : -1;
  const hovered = hover ? buckets[hover.index] : undefined;
  const hoverAt = (index: number, graph: HTMLElement) =>
    setHover({ index, bar: graph.children[index] });

  function applyRange(timeRange: { start: Date; end: Date } | undefined) {
    updateFilters((current) => ({ ...current, timeRange }));
  }
  function applyBuckets(from: number, to: number) {
    const range = rangeForBuckets(buckets, from, to);
    if (range) applyRange(range);
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
      <Header>
        <h2>Timeline</h2>
        <RangeText $applied={Boolean(activeRange)}>
          {shown &&
            `${formatEventTime(shown.start, zone)} – ${formatEventTime(shown.end, zone)} ${timeZoneLabel(zone)}`}
        </RangeText>
        {filters.timeRange && (
          <Button
            variant="subtle"
            disabled={disabled}
            onClick={() => {
              setSelection(null);
              applyRange(undefined);
            }}
          >
            Clear range
          </Button>
        )}
        <Button
          variant="subtle"
          aria-haspopup="dialog"
          aria-expanded={Boolean(rangeAnchor)}
          disabled={disabled || !shown}
          onClick={(event) => setRangeAnchor(rangeAnchor ? null : event.currentTarget)}
        >
          Custom range…
        </Button>
        {rangeAnchor && shown && (
          <CustomRange
            anchor={rangeAnchor}
            initial={filters.timeRange ?? shown}
            applied={Boolean(filters.timeRange)}
            zone={zone}
            onClose={() => setRangeAnchor(null)}
            onApply={(range) => {
              setSelection(null);
              applyRange(range);
              setRangeAnchor(null);
            }}
          />
        )}
      </Header>
      <Body>
        {loadError ? (
          <Notice $error role="alert">
            {loadError}
            <Button onClick={() => setRetry((value) => value + 1)}>Retry</Button>
          </Notice>
        ) : buckets.length > 0 && extent ? (
          <>
            <Graph
              $bins={buckets.length}
              aria-label={`Select a time range: ${total.toLocaleString()} events in ${buckets.length} intervals`}
              aria-describedby="timeline-help"
              tabIndex={0}
              onPointerDown={(event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                event.currentTarget.focus({ focusVisible: false });
                event.currentTarget.setPointerCapture(event.pointerId);
                const index = bucketAt(event.clientX, event.currentTarget);
                dragging.current = true;
                anchor.current = index;
                setHover(null);
                setCursor(index);
                setSelection([index, index]);
              }}
              onPointerMove={(event) => {
                const index = bucketAt(event.clientX, event.currentTarget);
                if (!dragging.current) {
                  if (hover?.index !== index) hoverAt(index, event.currentTarget);
                  return;
                }
                setCursor(index);
                setSelection([anchor.current, index]);
              }}
              onPointerLeave={() => setHover(null)}
              onPointerUp={(event) => {
                if (!dragging.current) return;
                dragging.current = false;
                applyBuckets(anchor.current, bucketAt(event.clientX, event.currentTarget));
                setSelection(null);
              }}
              onPointerCancel={() => {
                dragging.current = false;
                setSelection(null);
              }}
              onBlur={() => setHover(null)}
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
                hoverAt(next, event.currentTarget);
                setSelection([anchor.current, next]);
              }}
            >
              {/* Behind the bars: explicit columns keep them in place. */}
              {first >= 0 && last >= first && (
                <Selection style={{ gridColumn: `${first + 1} / ${last + 2}` }} />
              )}
              {buckets.map((bucket, index) => {
                const height = bucket.count ? Math.max(2, (bucket.count / peak) * GRAPH_HEIGHT) : 0;
                return (
                  <Bar
                    key={bucket.start}
                    style={{ gridColumn: index + 1 }}
                    data-hover={hover?.index === index || undefined}
                  >
                    {LEVELS.map(({ key }) =>
                      bucket[key] ? (
                        <div
                          key={key}
                          style={{
                            flexShrink: 0,
                            height: Math.max(1, (height * bucket[key]) / bucket.count),
                            background: colors[key],
                          }}
                        />
                      ) : null,
                    )}
                  </Bar>
                );
              })}
            </Graph>
            <Axis aria-hidden="true">
              {[0, 1, 2, 3].map((tick) => (
                <span
                  key={tick}
                  style={{
                    left: `${(tick / 3) * 100}%`,
                    transform: `translateX(-${(tick / 3) * 100}%)`,
                  }}
                >
                  {shortTime(
                    extent.start.getTime() +
                      ((extent.end.getTime() - extent.start.getTime()) * tick) / 3,
                    zone,
                  )}
                </span>
              ))}
            </Axis>
          </>
        ) : (
          <Notice as="output">
            {disabled ? "Available after import." : loading ? "Loading…" : "No timestamps match."}
          </Notice>
        )}
      </Body>
      <p id="timeline-help" hidden>
        Drag to select a range. Arrow keys move; Shift extends; Enter applies. The end is excluded.
      </p>
      {hovered && hover?.bar instanceof HTMLElement && (
        <Tip anchor={hover.bar} align="center" onClose={() => setHover(null)} role="tooltip">
          {formatEventTime(hovered.start, zone)} – {formatEventTime(hovered.end, zone)}
          <dl>
            {LEVELS.toReversed().map(({ key, label }) => (
              <div key={key} style={{ display: "contents" }}>
                <dt>
                  <Swatch $color={colors[key]} />
                  {label}
                </dt>
                <dd>{hovered[key].toLocaleString()}</dd>
              </div>
            ))}
            <dt>Total</dt>
            <dd>{hovered.count.toLocaleString()}</dd>
          </dl>
        </Tip>
      )}
    </Timeline>
  );
}
