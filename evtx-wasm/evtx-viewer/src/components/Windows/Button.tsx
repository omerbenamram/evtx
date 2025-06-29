import React from 'react';
import styled, { css } from 'styled-components';

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'secondary' | 'subtle';
  size?: 'small' | 'medium' | 'large';
  fullWidth?: boolean;
  icon?: React.ReactNode;
}

const sizeStyles = {
  small: css`
    padding: 4px 12px;
    font-size: ${({ theme }) => theme.fontSize.caption};
    min-height: 24px;
  `,
  medium: css`
    padding: 6px 16px;
    font-size: ${({ theme }) => theme.fontSize.body};
    min-height: 32px;
  `,
  large: css`
    padding: 8px 20px;
    font-size: ${({ theme }) => theme.fontSize.subtitle};
    min-height: 40px;
  `
};

const variantStyles = {
  primary: css`
    background-color: ${({ theme }) => theme.colors.accent.primary};
    color: ${({ theme }) => theme.colors.text.white};
    border: 1px solid ${({ theme }) => theme.colors.accent.primary};

    &:hover:not(:disabled) {
      background-color: ${({ theme }) => theme.colors.accent.hover};
      border-color: ${({ theme }) => theme.colors.accent.hover};
    }

    &:active:not(:disabled) {
      background-color: ${({ theme }) => theme.colors.accent.active};
      border-color: ${({ theme }) => theme.colors.accent.active};
    }
  `,
  secondary: css`
    background-color: ${({ theme }) => theme.colors.background.secondary};
    color: ${({ theme }) => theme.colors.text.primary};
    border: 1px solid ${({ theme }) => theme.colors.border.medium};

    &:hover:not(:disabled) {
      background-color: ${({ theme }) => theme.colors.background.hover};
      border-color: ${({ theme }) => theme.colors.border.dark};
    }

    &:active:not(:disabled) {
      background-color: ${({ theme }) => theme.colors.background.active};
      border-color: ${({ theme }) => theme.colors.accent.primary};
    }
  `,
  subtle: css`
    background-color: transparent;
    color: ${({ theme }) => theme.colors.text.primary};
    border: 1px solid transparent;

    &:hover:not(:disabled) {
      background-color: ${({ theme }) => theme.colors.background.hover};
      border-color: ${({ theme }) => theme.colors.border.light};
    }

    &:active:not(:disabled) {
      background-color: ${({ theme }) => theme.colors.background.active};
      border-color: ${({ theme }) => theme.colors.border.medium};
    }
  `
};

const StyledButton = styled.button<ButtonProps>`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: ${({ theme }) => theme.spacing.sm};
  font-family: ${({ theme }) => theme.fonts.body};
  font-weight: 400;
  line-height: 1.5;
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  cursor: pointer;
  transition: all ${({ theme }) => theme.transitions.fast};
  user-select: none;
  outline: none;
  position: relative;
  white-space: nowrap;

  ${({ size = 'medium' }) => sizeStyles[size]}
  ${({ variant = 'secondary' }) => variantStyles[variant]}
  ${({ fullWidth }) => fullWidth && css`
    width: 100%;
  `}

  &:focus-visible {
    box-shadow: 0 0 0 2px ${({ theme }) => theme.colors.accent.primary};
  }

  &:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* Ripple effect on click */
  &::after {
    content: '';
    position: absolute;
    top: 50%;
    left: 50%;
    width: 0;
    height: 0;
    border-radius: 50%;
    background-color: rgba(0, 0, 0, 0.1);
    transform: translate(-50%, -50%);
    transition: width 0.3s, height 0.3s;
  }

  &:active::after {
    width: 100%;
    height: 100%;
  }
`;

const IconWrapper = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
`;

export const Button: React.FC<ButtonProps> = ({ children, icon, ...props }) => {
  return (
    <StyledButton {...props}>
      {icon && <IconWrapper>{icon}</IconWrapper>}
      {children}
    </StyledButton>
  );
};