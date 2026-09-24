import { styled } from "styled-components";

export const ProgressBar = styled.progress.attrs({ max: 1, "aria-label": "Import progress" })`
  display: block;
  width: 100%;
  height: 8px;
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  border-radius: 0;
  appearance: none;
  accent-color: ${({ theme }) => theme.colors.accent.primary};
  background: ${({ theme }) => theme.colors.background.tertiary};
  &::-webkit-progress-bar {
    background: ${({ theme }) => theme.colors.background.tertiary};
  }
  &::-webkit-progress-value {
    background: ${({ theme }) => theme.colors.accent.primary};
  }
  &::-moz-progress-bar {
    background: ${({ theme }) => theme.colors.accent.primary};
  }
`;
