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

const Handle = styled.hr<{ $vertical: boolean }>`
  align-self: stretch;
  flex: 0 0 6px;
  width: ${({ $vertical }) => ($vertical ? "6px" : "auto")};
  height: ${({ $vertical }) => ($vertical ? "auto" : "6px")};
  margin: 0;
  padding: 0;
  border: 0;
  cursor: ${({ $vertical }) => ($vertical ? "col-resize" : "row-resize")};
  touch-action: none;
  background: transparent;
  &:hover,
  &:focus-visible {
    background: ${({ theme }) => theme.colors.selection.background};
  }
  &:focus-visible {
    outline: 1px solid ${({ theme }) => theme.colors.accent.primary};
    outline-offset: -1px;
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
