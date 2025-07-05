import React from "react";
import styled, { keyframes } from "styled-components";

const spin = keyframes`
  to { transform: rotate(360deg); }
`;

const SpinnerWrapper = styled.div<{ size?: number }>`
  width: ${({ size = 24 }) => size}px;
  height: ${({ size = 24 }) => size}px;
  border: 3px solid ${({ theme }) => theme.colors.border.light};
  border-top-color: ${({ theme }) => theme.colors.accent.primary};
  border-radius: 50%;
  animation: ${spin} 0.8s linear infinite;
`;

export const Spinner: React.FC<{
  size?: number;
  style?: React.CSSProperties;
}> = ({ size = 24, style }) => {
  return <SpinnerWrapper size={size} style={style} />;
};
