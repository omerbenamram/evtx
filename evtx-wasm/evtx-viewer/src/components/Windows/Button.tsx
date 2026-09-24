import type { ButtonHTMLAttributes, ReactNode } from "react";
import { css, styled } from "styled-components";

type ButtonVariant = "subtle" | "standard" | "accent";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** subtle: 24px, no border until hover (toolbars, inline actions). standard, accent: 28px. */
  variant?: ButtonVariant;
  /** Pressed state of a toggle button. */
  active?: boolean;
  icon?: ReactNode;
}

const variants = {
  subtle: css<{ $active: boolean }>`
    height: ${({ theme }) => theme.size.toolbarControl};
    min-width: ${({ theme }) => theme.size.toolbarControl};
    padding: 0 6px;
    border-color: transparent;
    background: ${({ theme, $active }) => ($active ? theme.colors.fill.selected : "transparent")};
    &:hover:not(:disabled) {
      background: ${({ theme, $active }) =>
        $active ? theme.colors.fill.selected : theme.colors.fill.hover};
    }
    &:active:not(:disabled) {
      background: ${({ theme }) => theme.colors.fill.pressed};
    }
  `,
  standard: css<{ $active: boolean }>`
    height: ${({ theme }) => theme.size.control};
    padding: 0 12px;
    border-color: ${({ theme }) => theme.colors.stroke.control};
    background: ${({ theme, $active }) =>
      $active ? theme.colors.fill.selected : theme.colors.surface.pane};
    &:hover:not(:disabled) {
      background-image: linear-gradient(
        ${({ theme }) => theme.colors.fill.hover},
        ${({ theme }) => theme.colors.fill.hover}
      );
    }
    &:active:not(:disabled) {
      color: ${({ theme }) => theme.colors.text.secondary};
    }
  `,
  accent: css`
    height: ${({ theme }) => theme.size.control};
    padding: 0 12px;
    border-color: transparent;
    background: ${({ theme }) => theme.colors.accent.rest};
    color: ${({ theme }) => theme.colors.accent.text};
    &:hover:not(:disabled) {
      background: ${({ theme }) => theme.colors.accent.hover};
    }
    &:active:not(:disabled) {
      background: ${({ theme }) => theme.colors.accent.pressed};
    }
    &:disabled {
      background: ${({ theme }) => theme.colors.stroke.control};
      color: ${({ theme }) => theme.colors.surface.pane};
    }
  `,
};

const Control = styled.button<{ $variant: ButtonVariant; $active: boolean }>`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  gap: 6px;
  border: 1px solid;
  border-radius: ${({ theme }) => theme.radius.control};
  color: ${({ theme }) => theme.colors.text.primary};
  white-space: nowrap;
  cursor: default;
  &:disabled {
    color: ${({ theme }) => theme.colors.text.tertiary};
  }
  ${({ $variant }) => variants[$variant]}
  &:focus-visible {
    outline-offset: 1px;
  }
  > span {
    display: inline-flex;
  }
`;

export function Button({
  children,
  icon,
  variant = "standard",
  active = false,
  type = "button",
  ...props
}: ButtonProps) {
  return (
    <Control type={type} $variant={variant} $active={active} {...props}>
      {icon && <span aria-hidden="true">{icon}</span>}
      {children}
    </Control>
  );
}
