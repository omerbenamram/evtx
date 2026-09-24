import { styled } from "styled-components";

export const ProgressBar = styled.progress.attrs({ max: 1, "aria-label": "Import progress" })`
  display: block;
  width: 100%;
  height: 3px;
  border: 0;
  border-radius: 2px;
  overflow: hidden;
  appearance: none;
  background: ${({ theme }) => theme.colors.stroke.control};
  &::-webkit-progress-bar {
    background: ${({ theme }) => theme.colors.stroke.control};
  }
  &::-webkit-progress-value {
    background: ${({ theme }) => theme.colors.accent.rest};
  }
  &::-moz-progress-bar {
    background: ${({ theme }) => theme.colors.accent.rest};
  }
`;
