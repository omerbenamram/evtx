import { useCallback, useRef, useState } from "react";
import { styled } from "styled-components";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";

export interface MenuItem {
  id: string;
  label: string;
  submenu: ContextMenuItem[];
}
export interface MenuBarProps {
  items: MenuItem[];
}

const Bar = styled.nav`
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
  height: 32px;
  padding: 0 4px;
  background: ${({ theme }) => theme.colors.surface.base};
`;

const MenuButton = styled.button<{ $open: boolean }>`
  height: 28px;
  padding: 0 10px;
  border: 0;
  border-radius: ${({ theme }) => theme.radius.control};
  background: ${({ theme, $open }) => ($open ? theme.colors.fill.hover : "transparent")};
  cursor: default;
  &:hover {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  &:focus-visible {
    outline-offset: -2px;
  }
`;

export function MenuBar({ items }: MenuBarProps) {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const [open, setOpen] = useState<{ index: number; button: HTMLButtonElement } | null>(null);
  const close = useCallback(() => setOpen(null), []);
  const menu = open && items[open.index];
  const step = (from: number, direction: number) =>
    (from + direction + items.length) % items.length;
  const show = (index: number) => {
    const button = buttons.current[index];
    if (button) setOpen({ index, button });
  };
  return (
    <Bar aria-label="Application menu">
      {items.map((item, index) => (
        <MenuButton
          key={item.id}
          ref={(el) => {
            buttons.current[index] = el;
          }}
          type="button"
          $open={open?.index === index}
          aria-haspopup="menu"
          aria-expanded={open?.index === index}
          onClick={() => (open?.index === index ? close() : show(index))}
          onMouseEnter={() => {
            if (open && open.index !== index) show(index);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              show(index);
            }
            if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
              event.preventDefault();
              buttons.current[step(index, event.key === "ArrowRight" ? 1 : -1)]?.focus();
            }
          }}
        >
          {item.label}
        </MenuButton>
      ))}
      {open && menu && (
        <ContextMenu
          key={menu.id}
          items={menu.submenu}
          position={open.button}
          onClose={close}
          ariaLabel={menu.label}
          onSwitchMenu={(direction) => {
            const next = step(open.index, direction);
            buttons.current[next]?.focus({ preventScroll: true });
            show(next);
          }}
        />
      )}
    </Bar>
  );
}
