import { useCallback, useState, type MouseEvent } from "react";
import { styled } from "styled-components";
import { Button } from "./Button";
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
  gap: 2px;
  flex-shrink: 0;
  padding: 0 4px;
  background: ${({ theme }) => theme.colors.background.tertiary};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
`;

export function MenuBar({ items }: MenuBarProps) {
  const [open, setOpen] = useState<{
    id: string;
    button: HTMLButtonElement;
    x: number;
    y: number;
  } | null>(null);
  const close = useCallback(() => setOpen(null), []);
  const menu = items.find((item) => item.id === open?.id);
  function show(id: string, button: HTMLButtonElement) {
    const bounds = button.getBoundingClientRect();
    setOpen({ id, button, x: bounds.left, y: bounds.bottom });
  }
  function hover(id: string, event: MouseEvent<HTMLButtonElement>) {
    if (open && open.id !== id) show(id, event.currentTarget);
  }
  return (
    <Bar aria-label="Application menu">
      {items.map((item) => (
        <Button
          key={item.id}
          variant="subtle"
          size="small"
          active={open?.id === item.id}
          aria-haspopup="menu"
          aria-expanded={open?.id === item.id}
          onClick={(event) => (open?.id === item.id ? close() : show(item.id, event.currentTarget))}
          onMouseEnter={(event) => hover(item.id, event)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              show(item.id, event.currentTarget);
            }
            if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
              event.preventDefault();
              const index = items.indexOf(item);
              const next =
                (index + (event.key === "ArrowRight" ? 1 : items.length - 1)) % items.length;
              event.currentTarget.parentElement?.querySelectorAll("button").item(next)?.focus();
            }
          }}
        >
          {item.label}
        </Button>
      ))}
      {open && menu && (
        <ContextMenu
          items={menu.submenu}
          position={open}
          onClose={close}
          returnFocus={open.button}
          ariaLabel={menu.label}
        />
      )}
    </Bar>
  );
}
