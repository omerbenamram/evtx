import React, { useLayoutEffect, useRef } from "react";
import { styled } from "styled-components";
import { Checkmark16Regular } from "@fluentui/react-icons";
import { Popover, type PopoverAnchor } from "./Popover";

export type ContextMenuItem =
  | { id: string; separator: true }
  | {
      id: string;
      label: string;
      icon?: React.ReactNode;
      shortcut?: string;
      /** Defined = a checkable item; shows a checkmark (or a dot with `radio`) when true. */
      checked?: boolean;
      radio?: boolean;
      disabled?: boolean;
      onClick?: () => void;
      separator?: false;
    };

export interface ContextMenuProps {
  items: ContextMenuItem[];
  /** A point (pointer position) or the trigger element to anchor below. */
  position: PopoverAnchor;
  onClose: () => void;
  returnFocus?: HTMLElement | null;
  ariaLabel?: string;
  /** ArrowLeft / ArrowRight, used by the menu bar to move between menus. */
  onSwitchMenu?: (direction: -1 | 1) => void;
}

const List = styled.div`
  min-width: 160px;
  max-width: 420px;
  outline: none;
`;

const Item = styled.button`
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 28px;
  padding: 6px 12px 6px 8px;
  border: 0;
  border-radius: ${({ theme }) => theme.radius.control};
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: default;
  overflow-wrap: anywhere;
  &:focus {
    outline: none;
  }
  /* The pointer moves focus too (see onPointerMove), so mouse and keyboard share one fill. */
  &:hover:not(:disabled),
  &:focus-visible {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  &:active:not(:disabled) {
    background: ${({ theme }) => theme.colors.fill.pressed};
  }
  &:disabled {
    color: ${({ theme }) => theme.colors.text.tertiary};
  }
  @media (forced-colors: active) {
    &:focus-visible {
      outline: 2px solid Highlight;
      outline-offset: -2px;
    }
  }
`;

const Glyph = styled.span`
  display: inline-flex;
  flex: 0 0 16px;
  align-items: center;
  justify-content: center;
  height: 16px;
`;

const Dot = styled.span`
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
`;

const Shortcut = styled.span`
  margin-left: auto;
  padding-left: 24px;
  color: ${({ theme }) => theme.colors.text.tertiary};
  white-space: nowrap;
`;

const Separator = styled.hr`
  height: 1px;
  border: 0;
  margin: 4px -4px;
  background: ${({ theme }) => theme.colors.stroke.divider};
`;

export function ContextMenu({
  items,
  position,
  onClose,
  returnFocus,
  ariaLabel = "Actions",
  onSwitchMenu,
}: ContextMenuProps) {
  const listRef = useRef<HTMLDivElement>(null);
  const enabled = () =>
    Array.from(listRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);

  // Runs after Popover has opened (child effects first). Also refocuses when items arrive
  // asynchronously (the column filter menu loads its values after opening).
  useLayoutEffect(() => {
    const list = listRef.current;
    const focused = document.activeElement;
    if (!list || (focused !== list && list.contains(focused))) return;
    list.focus({ preventScroll: true });
    if (items.some((item) => !item.separator && !item.disabled))
      list
        .querySelector<HTMLButtonElement>("button:not(:disabled)")
        ?.focus({ preventScroll: true });
  }, [items]);

  const navigate = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Tab") {
      event.preventDefault();
      onClose();
      return;
    }
    if (onSwitchMenu && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
      event.preventDefault();
      onSwitchMenu(event.key === "ArrowRight" ? 1 : -1);
      return;
    }
    const buttons = enabled();
    if (!buttons.length) return;
    const current = buttons.findIndex((button) => button === document.activeElement);
    let next: number;
    if (event.key === "ArrowDown") next = (current + 1) % buttons.length;
    else if (event.key === "ArrowUp") next = (current + buttons.length - 1) % buttons.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = buttons.length - 1;
    else if (event.key.length === 1 && event.key !== " " && !event.ctrlKey && !event.metaKey) {
      // Type-ahead: next item whose label starts with the typed character.
      const key = event.key.toLowerCase();
      const order = [...buttons.slice(current + 1), ...buttons.slice(0, current + 1)];
      const match = order.find((button) =>
        button.textContent?.trim().toLowerCase().startsWith(key),
      );
      if (!match) return;
      next = buttons.indexOf(match);
    } else return;
    event.preventDefault();
    event.stopPropagation();
    buttons[next]?.focus();
  };

  const hasCheck = items.some((item) => !item.separator && item.checked !== undefined);
  const hasIcon = items.some((item) => !item.separator && item.icon);

  return (
    <Popover anchor={position} onClose={onClose} returnFocus={returnFocus}>
      <List
        ref={listRef}
        role="menu"
        tabIndex={-1}
        aria-label={ariaLabel}
        onKeyDown={navigate}
        onContextMenu={(event) => event.preventDefault()}
        onPointerLeave={() => listRef.current?.focus({ preventScroll: true })}
      >
        {items.map((item) =>
          item.separator ? (
            <Separator key={item.id} />
          ) : (
            <Item
              key={item.id}
              type="button"
              role={
                item.checked === undefined
                  ? "menuitem"
                  : item.radio
                    ? "menuitemradio"
                    : "menuitemcheckbox"
              }
              aria-checked={item.checked}
              disabled={item.disabled}
              tabIndex={-1}
              onPointerMove={(event) => {
                if (document.activeElement !== event.currentTarget)
                  event.currentTarget.focus({ preventScroll: true });
              }}
              onClick={() => {
                onClose();
                item.onClick?.();
              }}
            >
              {hasCheck && (
                <Glyph aria-hidden="true">
                  {item.checked && (item.radio ? <Dot /> : <Checkmark16Regular />)}
                </Glyph>
              )}
              {hasIcon && <Glyph aria-hidden="true">{item.icon}</Glyph>}
              <span>{item.label}</span>
              {item.shortcut && <Shortcut>{item.shortcut}</Shortcut>}
            </Item>
          ),
        )}
      </List>
    </Popover>
  );
}
