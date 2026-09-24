import { styled, css } from "styled-components";

const fieldStyle = css<{ $compact?: boolean }>`
  min-width: 0;
  min-height: ${({ theme, $compact }) => theme.controlHeight[$compact ? "small" : "medium"]};
  padding: 3px 8px;
  border: 1px solid ${({ theme }) => theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  color: ${({ theme }) => theme.colors.text.primary};
  background: ${({ theme }) => theme.colors.background.secondary};
  font: inherit;
  line-height: 1.25;
  &::placeholder {
    color: ${({ theme }) => theme.colors.text.secondary};
  }
  &:disabled {
    opacity: 0.5;
    cursor: default;
  }
`;

export const Input = styled.input<{ $compact?: boolean }>`
  ${fieldStyle}
`;

export const Select = styled.select<{ $compact?: boolean }>`
  ${fieldStyle}
  max-width: 100%;
`;

export const SearchContainer = styled.div<{ $compact?: boolean }>`
  ${fieldStyle}
  display: flex;
  align-items: center;
  gap: 6px;
  &:focus-within {
    outline: 2px solid ${({ theme }) => theme.colors.accent.primary};
    outline-offset: -2px;
  }
  svg {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }
`;

export const SearchInput = styled(Input)`
  flex: 1;
  width: 100%;
  min-height: 24px;
  padding: 0;
  border: 0;
  background: transparent;
  &:focus-visible {
    outline: none;
  }
`;
