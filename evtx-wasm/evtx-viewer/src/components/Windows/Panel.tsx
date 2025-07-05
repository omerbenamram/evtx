import React from "react";
import styled, { css } from "styled-components";

export interface PanelProps {
  children: React.ReactNode;
  elevation?: "flat" | "raised" | "elevated";
  padding?: "none" | "small" | "medium" | "large";
  fullHeight?: boolean;
  className?: string;
  style?: React.CSSProperties;
}

// Transient prop names (prefixed with $) so styled-components filters them
interface StyledProps {
  $elevation: "flat" | "raised" | "elevated";
  $padding: "none" | "small" | "medium" | "large";
  $fullHeight?: boolean;
}

const elevationStyles = {
  flat: css`
    box-shadow: none;
    border: 1px solid ${({ theme }) => theme.colors.border.light};
  `,
  raised: css`
    box-shadow: ${({ theme }) => theme.shadows.sm};
    border: 1px solid ${({ theme }) => theme.colors.border.light};
  `,
  elevated: css`
    box-shadow: ${({ theme }) => theme.shadows.lg};
    border: none;
  `,
};

const paddingStyles = {
  none: css`
    padding: 0;
  `,
  small: css`
    padding: ${({ theme }) => theme.spacing.sm};
  `,
  medium: css`
    padding: ${({ theme }) => theme.spacing.md};
  `,
  large: css`
    padding: ${({ theme }) => theme.spacing.lg};
  `,
};

const StyledPanel = styled.div.withConfig({
  shouldForwardProp: (prop) =>
    !["$elevation", "$padding", "$fullHeight"].includes(prop as string),
})<StyledProps>`
  background-color: ${({ theme }) => theme.colors.background.secondary};
  border-radius: ${({ theme }) => theme.borderRadius.md};
  position: relative;

  ${({ $elevation }) => elevationStyles[$elevation]}
  ${({ $padding }) => paddingStyles[$padding]}
  ${({ $fullHeight }) =>
    $fullHeight &&
    css`
      height: 100%;
    `}
`;

export const Panel: React.FC<PanelProps> = ({
  children,
  elevation = "raised",
  padding = "medium",
  fullHeight,
  ...rest
}) => {
  return (
    <StyledPanel
      $elevation={elevation}
      $padding={padding}
      $fullHeight={fullHeight}
      {...rest}
    >
      {children}
    </StyledPanel>
  );
};

// Panel Header component
export interface PanelHeaderProps {
  children: React.ReactNode;
  actions?: React.ReactNode;
  noBorder?: boolean;
}

export const PanelHeader = styled.div<{ noBorder?: boolean }>`
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: ${({ theme }) => theme.spacing.md} ${({ theme }) => theme.spacing.lg};
  border-bottom: ${({ theme, noBorder }) =>
    noBorder ? "none" : `1px solid ${theme.colors.border.light}`};

  h1,
  h2,
  h3,
  h4,
  h5,
  h6 {
    margin: 0;
    font-weight: 600;
    color: ${({ theme }) => theme.colors.text.primary};
  }
`;

// Panel Body component
export const PanelBody = styled.div`
  padding: ${({ theme }) => theme.spacing.lg};
`;

// Panel Footer component
export const PanelFooter = styled.div`
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: ${({ theme }) => theme.spacing.sm};
  padding: ${({ theme }) => theme.spacing.md} ${({ theme }) => theme.spacing.lg};
  border-top: 1px solid ${({ theme }) => theme.colors.border.light};
  background-color: ${({ theme }) => theme.colors.background.tertiary};
  border-bottom-left-radius: ${({ theme }) => theme.borderRadius.md};
  border-bottom-right-radius: ${({ theme }) => theme.borderRadius.md};
`;
