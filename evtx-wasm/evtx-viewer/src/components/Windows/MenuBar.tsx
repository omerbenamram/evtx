import React, { useState, useRef, useEffect, useCallback } from "react";
import styled, { css } from "styled-components";

export interface MenuItem {
  id: string;
  label: string;
  onClick?: () => void;
  submenu?: MenuItem[];
  disabled?: boolean;
  separator?: boolean;
  icon?: React.ReactNode;
  shortcut?: string;
}

export interface MenuBarProps {
  items: MenuItem[];
  className?: string;
}

const MenuBarContainer = styled.div`
  display: flex;
  align-items: center;
  background-color: ${({ theme }) => theme.colors.background.tertiary};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  font-family: ${({ theme }) => theme.fonts.body};
  font-size: ${({ theme }) => theme.fontSize.body};
  user-select: none;
  position: relative;
  z-index: 1000;
`;

const MenuBarItem = styled.div<{ $isOpen?: boolean }>`
  padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.md};
  cursor: pointer;
  position: relative;
  transition: background-color ${({ theme }) => theme.transitions.fast};

  &:hover {
    background-color: ${({ theme }) => theme.colors.background.hover};
  }

  ${({ $isOpen, theme }) =>
    $isOpen &&
    css`
      background-color: ${theme.colors.background.active};
    `}
`;

const Dropdown = styled.div<{ $isOpen: boolean }>`
  position: absolute;
  top: 100%;
  left: 0;
  min-width: 200px;
  background-color: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  box-shadow: ${({ theme }) => theme.shadows.elevation};
  padding: ${({ theme }) => theme.spacing.xs} 0;
  opacity: ${({ $isOpen }) => ($isOpen ? 1 : 0)};
  visibility: ${({ $isOpen }) => ($isOpen ? "visible" : "hidden")};
  transform: ${({ $isOpen }) =>
    $isOpen ? "translateY(0)" : "translateY(-4px)"};
  transition: all ${({ theme }) => theme.transitions.fast};
  z-index: 1001;
`;

const DropdownItem = styled.div<{ $disabled?: boolean; $hasSubmenu?: boolean }>`
  display: flex;
  align-items: center;
  padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.lg};
  cursor: ${({ $disabled }) => ($disabled ? "not-allowed" : "pointer")};
  opacity: ${({ $disabled }) => ($disabled ? 0.5 : 1)};
  position: relative;

  &:hover {
    background-color: ${({ theme, $disabled }) =>
      $disabled ? "inherit" : theme.colors.selection.background};
  }

  ${({ $hasSubmenu }) =>
    $hasSubmenu &&
    css`
      &::after {
        content: "▶";
        position: absolute;
        right: 12px;
        font-size: 10px;
        color: ${({ theme }) => theme.colors.text.secondary};
      }
    `}
`;

const MenuIcon = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  margin-right: ${({ theme }) => theme.spacing.sm};
  color: ${({ theme }) => theme.colors.text.secondary};
`;

const MenuLabel = styled.span`
  flex: 1;
`;

const MenuShortcut = styled.span`
  margin-left: ${({ theme }) => theme.spacing.xl};
  color: ${({ theme }) => theme.colors.text.tertiary};
  font-size: ${({ theme }) => theme.fontSize.caption};
`;

const Separator = styled.div`
  height: 1px;
  background-color: ${({ theme }) => theme.colors.border.light};
  margin: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.md};
`;

const Submenu = styled(Dropdown)<{ $parentWidth: number }>`
  left: 100%;
  top: -4px;
  margin-left: 4px;
`;

interface MenuItemComponentProps {
  item: MenuItem;
  onClose: () => void;
  level?: number;
}

const MenuItemComponent: React.FC<MenuItemComponentProps> = ({
  item,
  onClose,
  level = 0,
}) => {
  const [submenuOpen, setSubmenuOpen] = useState(false);
  const itemRef = useRef<HTMLDivElement>(null);
  const [itemWidth, setItemWidth] = useState(0);

  useEffect(() => {
    if (itemRef.current) {
      setItemWidth(itemRef.current.offsetWidth);
    }
  }, []);

  const handleClick = useCallback(() => {
    if (!item.disabled && item.onClick) {
      item.onClick();
      onClose();
    }
  }, [item, onClose]);

  const handleMouseEnter = useCallback(() => {
    if (item.submenu && !item.disabled) {
      setSubmenuOpen(true);
    }
  }, [item]);

  const handleMouseLeave = useCallback(() => {
    setSubmenuOpen(false);
  }, []);

  if (item.separator) {
    return <Separator />;
  }

  return (
    <DropdownItem
      ref={itemRef}
      $disabled={item.disabled}
      $hasSubmenu={!!item.submenu}
      onClick={handleClick}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={handleMouseLeave}
    >
      {item.icon && <MenuIcon>{item.icon}</MenuIcon>}
      <MenuLabel>{item.label}</MenuLabel>
      {item.shortcut && <MenuShortcut>{item.shortcut}</MenuShortcut>}
      {item.submenu && (
        <Submenu $isOpen={submenuOpen} $parentWidth={itemWidth}>
          {item.submenu.map((subItem) => (
            <MenuItemComponent
              key={subItem.id}
              item={subItem}
              onClose={onClose}
              level={level + 1}
            />
          ))}
        </Submenu>
      )}
    </DropdownItem>
  );
};

export const MenuBar: React.FC<MenuBarProps> = ({ items, className }) => {
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const menuBarRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        menuBarRef.current &&
        !menuBarRef.current.contains(event.target as Node)
      ) {
        setOpenMenuId(null);
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  const handleMenuClick = useCallback(
    (itemId: string) => {
      setOpenMenuId(openMenuId === itemId ? null : itemId);
    },
    [openMenuId]
  );

  const handleMenuHover = useCallback(
    (itemId: string) => {
      if (openMenuId !== null) {
        setOpenMenuId(itemId);
      }
    },
    [openMenuId]
  );

  const handleClose = useCallback(() => {
    setOpenMenuId(null);
  }, []);

  return (
    <MenuBarContainer ref={menuBarRef} className={className}>
      {items.map((item) => (
        <MenuBarItem
          key={item.id}
          $isOpen={openMenuId === item.id}
          onClick={() => handleMenuClick(item.id)}
          onMouseEnter={() => handleMenuHover(item.id)}
        >
          {item.label}
          {item.submenu && (
            <Dropdown $isOpen={openMenuId === item.id}>
              {item.submenu.map((subItem) => (
                <MenuItemComponent
                  key={subItem.id}
                  item={subItem}
                  onClose={handleClose}
                />
              ))}
            </Dropdown>
          )}
        </MenuBarItem>
      ))}
    </MenuBarContainer>
  );
};
