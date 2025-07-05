import React, { useEffect, useRef } from "react";
import styled from "styled-components";

export interface ContextMenuItem {
  id: string;
  label: string;
  icon?: React.ReactNode;
  disabled?: boolean;
  onClick?: () => void;
}

export interface ContextMenuProps {
  items: ContextMenuItem[];
  /** Absolute viewport pixel coordinates */
  position: { x: number; y: number };
  onClose: () => void;
  className?: string;
  style?: React.CSSProperties;
}

const MenuContainer = styled.div<{ $x: number; $y: number }>`
  position: fixed;
  left: ${({ $x }) => $x}px;
  top: ${({ $y }) => $y}px;
  background-color: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  box-shadow: ${({ theme }) => theme.shadows.elevation};
  padding: ${({ theme }) => theme.spacing.xs} 0;
  z-index: 2000;
  min-width: 160px;
  user-select: none;
`;

const MenuItemRow = styled.div<{ $disabled?: boolean }>`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.sm};
  padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.lg};
  cursor: ${({ $disabled }) => ($disabled ? "not-allowed" : "pointer")};
  opacity: ${({ $disabled }) => ($disabled ? 0.5 : 1)};

  &:hover {
    background-color: ${({ theme, $disabled }) =>
      $disabled ? "inherit" : theme.colors.selection.background};
  }
`;

const ItemIcon = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  color: ${({ theme }) => theme.colors.text.secondary};
`;

const ItemLabel = styled.span`
  flex: 1;
`;

export const ContextMenu: React.FC<ContextMenuProps> = ({
  items,
  position,
  onClose,
  className,
  style,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);

  // Close on outside click or Esc key
  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        onClose();
      }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };

    document.addEventListener("mousedown", handleMouseDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  return (
    <MenuContainer
      ref={containerRef}
      $x={position.x}
      $y={position.y}
      className={className}
      style={style}
    >
      {items.map((item) => (
        <MenuItemRow
          key={item.id}
          $disabled={item.disabled}
          onClick={() => {
            if (item.disabled) return;
            if (item.onClick) item.onClick();
            onClose();
          }}
        >
          {item.icon && <ItemIcon>{item.icon}</ItemIcon>}
          <ItemLabel>{item.label}</ItemLabel>
        </MenuItemRow>
      ))}
    </MenuContainer>
  );
};
