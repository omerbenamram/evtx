import type { ButtonHTMLAttributes, ReactNode } from "react";
import { styled } from "styled-components";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "secondary" | "subtle";
  size?: "small" | "medium";
  active?: boolean;
  icon?: ReactNode;
}

const Control = styled.button<{
  $variant: "secondary" | "subtle";
  $size: "small" | "medium";
  $active: boolean;
}>`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  gap: 6px;
  min-width: ${({ $size, theme }) => theme.controlHeight[$size]};
  min-height: ${({ $size, theme }) => theme.controlHeight[$size]};
  padding: 3px 10px;
  border: 1px solid
    ${({ theme, $variant, $active }) =>
      $active
        ? theme.colors.selection.border
        : $variant === "subtle"
          ? "transparent"
          : theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  background: ${({ theme, $variant, $active }) =>
    $active
      ? theme.colors.selection.background
      : $variant === "subtle"
        ? "transparent"
        : theme.colors.background.tertiary};
  color: ${({ theme }) => theme.colors.text.primary};
  font: inherit;
  line-height: 1.25;
  white-space: nowrap;
  cursor: pointer;

  &:hover:not(:disabled) {
    background: ${({ theme }) => theme.colors.background.hover};
    border-color: ${({ theme }) => theme.colors.border.dark};
  }
  &:active:not(:disabled) {
    background: ${({ theme }) => theme.colors.background.active};
  }
  &:disabled {
    opacity: 0.5;
    cursor: default;
  }
  svg {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }
`;

export function Button({
  children,
  icon,
  variant = "secondary",
  size = "medium",
  active = false,
  type = "button",
  ...props
}: ButtonProps) {
  return (
    <Control type={type} $variant={variant} $size={size} $active={active} {...props}>
      {icon && (
        <span aria-hidden="true" style={{ display: "inline-flex" }}>
          {icon}
        </span>
      )}
      {children}
    </Control>
  );
}
