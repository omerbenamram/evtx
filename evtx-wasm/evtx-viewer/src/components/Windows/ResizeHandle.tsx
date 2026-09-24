import { useRef, type CSSProperties, type PointerEvent } from "react";
import { styled } from "styled-components";

interface ResizeHandleProps {
  label: string;
  value: number;
  min: number;
  max: number;
  orientation: "vertical" | "horizontal";
  onResize: (value: number) => void;
  /** The settled value after a drag or key press, for handles whose onResize only previews. */
  onCommit?: (value: number) => void;
  reverse?: boolean;
  style?: CSSProperties;
}

// A 1px divider line with a 7px hit area: the -3px margins overlap the neighbouring panes,
// so docked panes meet edge to edge. The line turns accent on hover, drag and focus.
const Handle = styled.hr<{ $vertical: boolean }>`
  position: relative;
  z-index: 2;
  align-self: stretch;
  flex: 0 0 7px;
  width: ${({ $vertical }) => ($vertical ? "7px" : "auto")};
  height: ${({ $vertical }) => ($vertical ? "auto" : "7px")};
  margin: ${({ $vertical }) => ($vertical ? "0 -3px" : "-3px 0")};
  padding: 0;
  border: 0;
  cursor: ${({ $vertical }) => ($vertical ? "col-resize" : "row-resize")};
  touch-action: none;
  background: transparent;
  &::after {
    content: "";
    position: absolute;
    ${({ $vertical }) => ($vertical ? "inset: 0 3px;" : "inset: 3px 0;")}
    background: ${({ theme }) => theme.colors.stroke.divider};
  }
  &:hover::after,
  &:active::after,
  &:focus-visible::after {
    background: ${({ theme }) => theme.colors.accent.rest};
  }
  &:focus-visible {
    outline: none;
    &::after {
      ${({ $vertical }) => ($vertical ? "inset: 0 2px;" : "inset: 2px 0;")}
    }
  }
  @media (forced-colors: active) {
    &::after {
      background: CanvasText;
    }
    &:focus-visible::after {
      background: Highlight;
    }
  }
`;

export function ResizeHandle({
  label,
  value,
  min,
  max,
  orientation,
  onResize,
  onCommit,
  reverse = false,
  style,
}: ResizeHandleProps) {
  const drag = useRef<{ position: number; value: number; next: number } | null>(null);
  const vertical = orientation === "vertical";
  const clamp = (next: number) => Math.round(Math.max(min, Math.min(max, next)));
  function finish(event: PointerEvent<HTMLHRElement>, cancelled: boolean) {
    const current = drag.current;
    if (!current) return;
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId))
      event.currentTarget.releasePointerCapture(event.pointerId);
    const next = cancelled ? current.value : current.next;
    onResize(next);
    onCommit?.(next);
  }
  return (
    <Handle
      $vertical={vertical}
      style={style}
      tabIndex={0}
      aria-label={label}
      aria-orientation={orientation}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget.focus();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { position: vertical ? event.clientX : event.clientY, value, next: value };
      }}
      onPointerMove={(event) => {
        const current = drag.current;
        if (!current) return;
        const position = vertical ? event.clientX : event.clientY;
        current.next = clamp(current.value + (position - current.position) * (reverse ? -1 : 1));
        onResize(current.next);
      }}
      onPointerUp={(event) => finish(event, false)}
      onPointerCancel={(event) => finish(event, true)}
      onLostPointerCapture={(event) => finish(event, true)}
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        if (event.key === "Escape" && drag.current) {
          event.preventDefault();
          onResize(drag.current.value);
          onCommit?.(drag.current.value);
          drag.current = null;
          return;
        }
        const decrease = vertical ? "ArrowLeft" : "ArrowUp";
        const increase = vertical ? "ArrowRight" : "ArrowDown";
        if (![decrease, increase, "Home", "End"].includes(event.key)) return;
        event.preventDefault();
        event.stopPropagation();
        const delta =
          (event.key === increase ? 1 : -1) * (event.shiftKey ? 1 : 10) * (reverse ? -1 : 1);
        const next = event.key === "Home" ? min : event.key === "End" ? max : clamp(value + delta);
        onResize(next);
        onCommit?.(next);
      }}
    />
  );
}
