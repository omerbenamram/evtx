import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { styled } from "styled-components";
import { Checkmark16Regular } from "@fluentui/react-icons";

export interface ContextMenuItem {
  id: string;
  label: string;
  icon?: React.ReactNode;
  shortcut?: string;
  checked?: boolean;
  disabled?: boolean;
  onClick?: () => void;
}

export interface ContextMenuProps {
  items: ContextMenuItem[];
  position: { x: number; y: number };
  onClose: () => void;
  returnFocus?: HTMLElement | null;
  ariaLabel?: string;
}

const Menu = styled.div`
  position: fixed;
  z-index: 2000;
  min-width: min(180px, calc(100vw - 16px));
  max-width: calc(100vw - 16px);
  max-height: calc(100dvh - 16px);
  overflow: auto;
  box-sizing: border-box;
  padding: 3px 0;
  background: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  box-shadow: ${({ theme }) => theme.shadows.elevation};
  color: ${({ theme }) => theme.colors.text.primary};
`;

const Item = styled.button`
  display: flex;
  align-items: center;
  width: 100%;
  min-height: 28px;
  gap: 8px;
  padding: 4px 16px 4px 8px;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: default;
  white-space: normal;
  overflow-wrap: anywhere;
  &:hover:not(:disabled),
  &:focus-visible {
    background: ${({ theme }) => theme.colors.selection.background};
    outline: 1px solid ${({ theme }) => theme.colors.accent.primary};
    outline-offset: -1px;
  }
  &:disabled {
    color: ${({ theme }) => theme.colors.text.tertiary};
  }
`;

const Icon = styled.span`
  display: inline-flex;
  flex: 0 0 16px;
  width: 16px;
  align-items: center;
  justify-content: center;
`;

export function ContextMenu({
  items,
  position,
  onClose,
  returnFocus,
  ariaLabel = "Actions",
}: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  const [location, setLocation] = useState(position);
  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const place = () => {
      const bounds = menu.getBoundingClientRect();
      setLocation({
        x: Math.max(8, Math.min(position.x, window.innerWidth - bounds.width - 8)),
        y: Math.max(8, Math.min(position.y, window.innerHeight - bounds.height - 8)),
      });
    };
    place();
    const observer = new ResizeObserver(place);
    observer.observe(menu);
    return () => observer.disconnect();
  }, [position.x, position.y]);

  useLayoutEffect(() => {
    const invoker =
      returnFocus ??
      (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    const menu = menuRef.current;
    const first = menu?.querySelector<HTMLButtonElement>("button:not(:disabled)");
    (first ?? menu)?.focus({ preventScroll: true });
    const dismissOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !menu?.contains(event.target)) closeRef.current();
    };
    const dismissOnResize = () => closeRef.current();
    document.addEventListener("pointerdown", dismissOutside);
    window.addEventListener("resize", dismissOnResize);
    return () => {
      document.removeEventListener("pointerdown", dismissOutside);
      window.removeEventListener("resize", dismissOnResize);
      if (invoker?.isConnected) invoker.focus({ preventScroll: true });
    };
  }, [returnFocus]);

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (
      items.some((item) => !item.disabled) &&
      menu &&
      (document.activeElement === menu || !menu.contains(document.activeElement))
    ) {
      menu
        .querySelector<HTMLButtonElement>("button:not(:disabled)")
        ?.focus({ preventScroll: true });
    }
  }, [items]);

  const navigate = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    const buttons = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [],
    );
    if (!buttons.length) return;
    const current = buttons.findIndex((button) => button === document.activeElement);
    let next: number;
    switch (event.key) {
      case "ArrowDown":
        next = (current + 1) % buttons.length;
        break;
      case "ArrowUp":
        next = (current + buttons.length - 1) % buttons.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = buttons.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
    buttons[next]?.focus();
  };

  return createPortal(
    <Menu
      ref={menuRef}
      role="menu"
      tabIndex={-1}
      aria-label={ariaLabel}
      style={{ left: location.x, top: location.y }}
      onKeyDown={navigate}
      onContextMenu={(event) => event.preventDefault()}
    >
      {items.map((item) => (
        <Item
          key={item.id}
          type="button"
          role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
          aria-checked={item.checked}
          disabled={item.disabled}
          tabIndex={-1}
          onClick={() => {
            onClose();
            item.onClick?.();
          }}
        >
          <Icon aria-hidden="true">{item.checked ? <Checkmark16Regular /> : item.icon}</Icon>
          <span style={{ flex: 1 }}>{item.label}</span>
          {item.shortcut && <span style={{ marginLeft: 20, opacity: 0.7 }}>{item.shortcut}</span>}
        </Item>
      ))}
    </Menu>,
    document.body,
  );
}
