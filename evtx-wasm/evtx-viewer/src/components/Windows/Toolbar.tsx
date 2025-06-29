import React from "react";
import styled, { css } from "styled-components";

export interface ToolbarProps {
  children: React.ReactNode;
  variant?: "default" | "compact";
  position?: "top" | "bottom";
  className?: string;
}

export interface ToolbarButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  icon: React.ReactNode;
  label?: string;
  isActive?: boolean;
  showLabel?: boolean;
}

export interface ToolbarSeparatorProps {
  orientation?: "vertical" | "horizontal";
}

export interface ToolbarGroupProps {
  children: React.ReactNode;
  className?: string;
}

const StyledToolbar = styled.div<ToolbarProps>`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.xs};
  padding: ${({ theme, variant }) =>
    variant === "compact" ? theme.spacing.xs : theme.spacing.sm};
  background-color: ${({ theme }) => theme.colors.background.tertiary};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  min-height: ${({ variant }) => (variant === "compact" ? "32px" : "40px")};

  ${({ position }) =>
    position === "bottom" &&
    css`
      border-bottom: none;
      border-top: 1px solid ${({ theme }) => theme.colors.border.light};
    `}
`;

const StyledToolbarButton = styled.button<{
  $showLabel?: boolean;
  $isActive?: boolean;
}>`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: ${({ theme }) => theme.spacing.xs};
  padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) => theme.spacing.sm};
  min-width: ${({ $showLabel }) => ($showLabel ? "auto" : "32px")};
  height: 32px;
  background-color: transparent;
  border: 1px solid transparent;
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  color: ${({ theme }) => theme.colors.text.primary};
  font-family: ${({ theme }) => theme.fonts.body};
  font-size: ${({ theme }) => theme.fontSize.body};
  cursor: pointer;
  transition: all ${({ theme }) => theme.transitions.fast};
  user-select: none;
  outline: none;

  &:hover:not(:disabled) {
    background-color: ${({ theme }) => theme.colors.background.hover};
    border-color: ${({ theme }) => theme.colors.border.light};
  }

  &:active:not(:disabled) {
    background-color: ${({ theme }) => theme.colors.background.active};
    border-color: ${({ theme }) => theme.colors.border.medium};
  }

  ${({ $isActive, theme }) =>
    $isActive &&
    css`
      background-color: ${theme.colors.background.active};
      border-color: ${theme.colors.border.medium};

      &:hover:not(:disabled) {
        background-color: ${theme.colors.background.active};
        border-color: ${theme.colors.border.dark};
      }
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
    content: "";
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

const ToolbarIcon = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  color: ${({ theme }) => theme.colors.text.secondary};
`;

const ToolbarLabel = styled.span`
  white-space: nowrap;
`;

const StyledToolbarSeparator = styled.div<ToolbarSeparatorProps>`
  ${({ orientation = "vertical", theme }) =>
    orientation === "vertical"
      ? css`
          width: 1px;
          height: 20px;
          background-color: ${theme.colors.border.light};
          margin: 0 ${theme.spacing.xs};
        `
      : css`
          width: 100%;
          height: 1px;
          background-color: ${theme.colors.border.light};
          margin: ${theme.spacing.xs} 0;
        `}
`;

const StyledToolbarGroup = styled.div`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.xs};

  &:not(:last-child) {
    margin-right: ${({ theme }) => theme.spacing.sm};
  }
`;

// Spacer component to push items to the right
export const ToolbarSpacer = styled.div`
  flex: 1;
`;

export const Toolbar: React.FC<ToolbarProps> = ({ children, ...props }) => {
  return <StyledToolbar {...props}>{children}</StyledToolbar>;
};

export const ToolbarButton: React.FC<ToolbarButtonProps> = ({
  children,
  icon,
  label,
  showLabel = false,
  isActive = false,
  ...rest
}) => {
  return (
    <StyledToolbarButton $showLabel={showLabel} $isActive={isActive} {...rest}>
      <ToolbarIcon>{icon}</ToolbarIcon>
      {(showLabel || !icon) && label && <ToolbarLabel>{label}</ToolbarLabel>}
      {children}
    </StyledToolbarButton>
  );
};

export const ToolbarSeparator: React.FC<ToolbarSeparatorProps> = (props) => {
  return <StyledToolbarSeparator {...props} />;
};

export const ToolbarGroup: React.FC<ToolbarGroupProps> = ({
  children,
  ...props
}) => {
  return <StyledToolbarGroup {...props}>{children}</StyledToolbarGroup>;
};
