import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type ReactElement,
} from "react";
import { createPortal } from "react-dom";
import { keyframes, styled } from "styled-components";

/** A trigger element, or a point (for menus opened at the pointer). */
export type PopoverAnchor = HTMLElement | { x: number; y: number };

export interface PopoverProps extends HTMLAttributes<HTMLDivElement> {
  anchor: PopoverAnchor;
  /** Called on Esc, a pointer press outside the popover and its trigger. */
  onClose: () => void;
  align?: "start" | "center";
  /** Where focus goes on close if it was inside; defaults to the anchor element. */
  returnFocus?: HTMLElement | null;
}

const EDGE = 8;
const GAP = 4;

// Below-start by default; flips above when it does not fit below, shifts to stay on screen.
function place(el: HTMLElement, anchor: PopoverAnchor, align: "start" | "center") {
  const a =
    anchor instanceof HTMLElement
      ? anchor.getBoundingClientRect()
      : { left: anchor.x, top: anchor.y, bottom: anchor.y, width: 0 };
  const width = el.offsetWidth;
  const height = el.offsetHeight;
  const viewWidth = document.documentElement.clientWidth;
  const viewHeight = document.documentElement.clientHeight;
  const gap = anchor instanceof HTMLElement ? GAP : 0;
  const above = a.bottom + gap + height > viewHeight - EDGE && a.top - gap - height >= EDGE;
  const top = above ? a.top - gap - height : a.bottom + gap;
  const left = align === "center" ? a.left + a.width / 2 - width / 2 : a.left;
  el.style.left = `${Math.max(EDGE, Math.min(left, viewWidth - width - EDGE))}px`;
  el.style.top = `${Math.max(EDGE, Math.min(top, viewHeight - height - EDGE))}px`;
  el.dataset.side = above ? "top" : "bottom";
}

const enter = keyframes`
  from { opacity: 0; transform: translateY(var(--popover-slide)); }
`;

const Surface = styled.div`
  position: fixed;
  inset: auto;
  margin: 0;
  max-width: calc(100vw - ${EDGE * 2}px);
  max-height: calc(100dvh - ${EDGE * 2}px);
  overflow: auto;
  padding: 4px;
  border: 1px solid ${({ theme }) => theme.colors.stroke.control};
  border-radius: ${({ theme }) => theme.radius.flyout};
  background: ${({ theme }) => theme.colors.surface.pane};
  color: ${({ theme }) => theme.colors.text.primary};
  box-shadow: ${({ theme }) => theme.shadow.flyout};
  --popover-slide: -4px;
  &[data-side="top"] {
    --popover-slide: 4px;
  }
  animation: ${enter} ${({ theme }) => theme.motion.fast};
`;

/**
 * Shared flyout surface in the top layer (native `popover`), so it escapes scroll
 * containers and z-index stacks. Mounted = open; unmount to close.
 */
export function Popover({
  anchor,
  onClose,
  align = "start",
  returnFocus,
  children,
  ...props
}: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);
  const latest = useRef({ anchor, onClose, align, returnFocus });
  useLayoutEffect(() => {
    latest.current = { anchor, onClose, align, returnFocus };
    if (ref.current) place(ref.current, anchor, align);
  });

  useLayoutEffect(() => {
    const el = ref.current!;
    // ponytail: popover="manual" with our own dismiss; "auto" light dismiss would close on
    // the trigger's pointerdown and the trigger's click would reopen it.
    el.showPopover();
    const reposition = () => place(el, latest.current.anchor, latest.current.align);
    reposition();
    const observer = new ResizeObserver(reposition);
    observer.observe(el);
    const trigger = () => {
      const current = latest.current.anchor;
      return current instanceof HTMLElement ? current : null;
    };
    const outside = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && !el.contains(target) && !trigger()?.contains(target))
        latest.current.onClose();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (el.contains(document.activeElement)) {
        event.preventDefault();
        event.stopPropagation();
      }
      latest.current.onClose();
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", escape, true);
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      observer.disconnect();
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", escape, true);
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
      const back = latest.current.returnFocus ?? trigger();
      if (el.contains(document.activeElement) && back?.isConnected)
        back.focus({ preventScroll: true });
    };
  }, []);

  return createPortal(
    <Surface ref={ref} popover="manual" {...props}>
      {children}
    </Surface>,
    document.body,
  );
}

const TooltipSurface = styled(Popover)`
  max-width: 320px;
  padding: 5px 8px 6px;
  pointer-events: none;
  overflow-wrap: anywhere;
`;

/**
 * Shows `label` 500ms after pointer hover or keyboard focus on its single child.
 * The child keeps its own accessible name (aria-label); the tooltip is visual.
 */
export function Tooltip({ label, children }: { label: string; children: ReactElement }) {
  const wrapper = useRef<HTMLSpanElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const hide = () => {
    clearTimeout(timer.current);
    setAnchor(null);
  };
  const show = () => {
    const target = wrapper.current?.firstElementChild;
    clearTimeout(timer.current);
    if (target instanceof HTMLElement) timer.current = setTimeout(() => setAnchor(target), 500);
  };
  useEffect(() => () => clearTimeout(timer.current), []);
  return (
    <span
      ref={wrapper}
      style={{ display: "contents" }}
      onMouseEnter={show}
      onMouseLeave={hide}
      onFocus={(event) => {
        if (event.target.matches(":focus-visible")) show();
      }}
      onBlur={hide}
      onPointerDown={hide}
    >
      {children}
      {anchor && (
        <TooltipSurface anchor={anchor} align="center" onClose={hide} role="tooltip">
          {label}
        </TooltipSurface>
      )}
    </span>
  );
}
