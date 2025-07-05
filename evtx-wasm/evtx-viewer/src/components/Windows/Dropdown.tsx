import React, { useState, useRef, useCallback, useEffect } from "react";
import styled from "styled-components";
import { ChevronDown20Regular } from "@fluentui/react-icons";

export interface DropdownOption<T = string | number> {
  label: string;
  value: T;
  icon?: React.ReactNode;
  disabled?: boolean;
}

export interface DropdownProps<T = string | number> {
  options: DropdownOption<T>[];
  value: T;
  onChange: (value: T) => void;
  label?: string;
  disabled?: boolean;
  /** Optionally override the pop-over min-width */
  width?: number | string;
  className?: string;
  style?: React.CSSProperties;
}

const Wrapper = styled.div`
  position: relative;
  display: inline-block;
`;

const ToggleButton = styled.button<{ $disabled?: boolean }>`
  display: inline-flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.xs};
  padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.sm};
  min-height: 32px;
  background-color: ${({ theme }) => theme.colors.background.tertiary};
  color: ${({ theme }) => theme.colors.text.primary};
  border: 1px solid ${({ theme }) => theme.colors.border.light};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  cursor: ${({ $disabled }) => ($disabled ? "not-allowed" : "pointer")};
  font-family: ${({ theme }) => theme.fonts.body};
  font-size: ${({ theme }) => theme.fontSize.body};
  transition: all ${({ theme }) => theme.transitions.fast};

  &:hover:not(:disabled) {
    background-color: ${({ theme }) => theme.colors.background.hover};
  }

  &:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px ${({ theme }) => theme.colors.accent.primary};
  }
`;

const ValueLabel = styled.span`
  white-space: nowrap;
`;

const DropdownIcon = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: ${({ theme }) => theme.colors.text.secondary};
`;

const Menu = styled.div<{ $isOpen: boolean; $width?: number | string }>`
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  min-width: ${({ $width }) =>
    typeof $width === "number" ? `${$width}px` : $width || "100%"};
  background-color: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  box-shadow: ${({ theme }) => theme.shadows.elevation};
  z-index: 1001;
  opacity: ${({ $isOpen }) => ($isOpen ? 1 : 0)};
  visibility: ${({ $isOpen }) => ($isOpen ? "visible" : "hidden")};
  transform: ${({ $isOpen }) =>
    $isOpen ? "translateY(0)" : "translateY(-4px)"};
  transition: all ${({ theme }) => theme.transitions.fast};
  max-height: 240px;
  overflow-y: auto;
`;

const MenuItem = styled.div<{
  $isSelected?: boolean;
  $disabled?: boolean;
}>`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.sm};
  padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.md};
  cursor: ${({ $disabled }) => ($disabled ? "not-allowed" : "pointer")};
  color: ${({ theme, $disabled }) =>
    $disabled ? theme.colors.text.tertiary : theme.colors.text.primary};
  background-color: ${({ $isSelected, theme }) =>
    $isSelected ? theme.colors.selection.background : "transparent"};

  &:hover {
    background-color: ${({ theme, $disabled }) =>
      $disabled ? "transparent" : theme.colors.background.hover};
  }
`;

export function Dropdown<T = string | number>({
  options,
  value,
  onChange,
  label,
  disabled,
  width,
  className,
  style,
}: DropdownProps<T>) {
  const [open, setOpen] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);

  const toggleOpen = useCallback(() => {
    if (disabled) return;
    setOpen((prev) => !prev);
  }, [disabled]);

  const handleClickOutside = useCallback((e: MouseEvent) => {
    if (wrapperRef.current && !wrapperRef.current.contains(e.target as Node)) {
      setOpen(false);
    }
  }, []);

  useEffect(() => {
    if (open) {
      document.addEventListener("mousedown", handleClickOutside);
    } else {
      document.removeEventListener("mousedown", handleClickOutside);
    }
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [open, handleClickOutside]);

  const selected = options.find((o) => o.value === value);

  return (
    <Wrapper ref={wrapperRef} className={className} style={style}>
      <ToggleButton onClick={toggleOpen} $disabled={disabled}>
        {label && <ValueLabel>{label}:</ValueLabel>}
        <ValueLabel>{selected ? selected.label : ""}</ValueLabel>
        <DropdownIcon>
          <ChevronDown20Regular />
        </DropdownIcon>
      </ToggleButton>

      <Menu $isOpen={open} $width={width}>
        {options.map((opt) => (
          <MenuItem
            key={String(opt.value)}
            $isSelected={value === opt.value}
            $disabled={opt.disabled}
            onClick={() => {
              if (opt.disabled) return;
              onChange(opt.value);
              setOpen(false);
            }}
          >
            {opt.icon && <span>{opt.icon}</span>}
            <ValueLabel>{opt.label}</ValueLabel>
          </MenuItem>
        ))}
      </Menu>
    </Wrapper>
  );
}
