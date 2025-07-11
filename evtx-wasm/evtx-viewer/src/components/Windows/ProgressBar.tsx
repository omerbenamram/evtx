import React from "react";
import styled, { keyframes } from "styled-components";

export interface ProgressBarProps {
  /** 0 → 1 (will be clamped automatically). If undefined the bar shows an indeterminate animation. */
  value?: number;
  /** Optional descriptive text shown centred below the bar. */
  label?: string;
  /** Bar height in pixels (default 8). */
  height?: number;
  className?: string;
  style?: React.CSSProperties;
}

const Container = styled.div`
  width: 100%;
  background-color: ${({ theme }) => theme.colors.background.tertiary};
  border: 1px solid ${({ theme }) => theme.colors.border.light};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  overflow: hidden;
`;

const Filler = styled.div<{ $pct: number }>`
  width: ${({ $pct }) => $pct}%;
  height: 100%;
  background-color: ${({ theme }) => theme.colors.accent.primary};
  transition: width 0.25s ease-out;
`;

// Simple indeterminate stripes animation
const indeterminate = keyframes`
  0%   { left: -40%; width: 40%; }
  50%  { left: 20%;  width: 60%; }
  100% { left: 100%; width: 40%; }
`;

const IndeterminateFiller = styled.div`
  position: absolute;
  top: 0;
  bottom: 0;
  background-color: ${({ theme }) => theme.colors.accent.primary};
  animation: ${indeterminate} 1.5s infinite ease-in-out;
`;

const BarWrapper = styled.div<{ $height: number }>`
  position: relative;
  width: 100%;
  height: ${({ $height }) => $height}px;
`;

export const ProgressBar: React.FC<ProgressBarProps> = ({
  value,
  label,
  height = 8,
  className,
  style,
}) => {
  const pct = Math.max(0, Math.min(1, value ?? 0)) * 100;
  const indeterminateMode = value === undefined;

  return (
    <div className={className} style={style}>
      <Container>
        <BarWrapper $height={height}>
          {indeterminateMode ? <IndeterminateFiller /> : <Filler $pct={pct} />}
        </BarWrapper>
      </Container>
      {label && (
        <div
          style={{
            marginTop: 4,
            fontSize: 12,
            textAlign: "center",
          }}
        >
          {label}
        </div>
      )}
    </div>
  );
};
