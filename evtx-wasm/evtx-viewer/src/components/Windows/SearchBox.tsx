import { styled, css } from "styled-components";

// WinUI text field: 1px stroke with a darker bottom edge that turns into a 2px accent line on focus.
const fieldStyle = css`
  min-width: 0;
  height: ${({ theme }) => theme.size.control};
  padding: 0 8px;
  border: 1px solid ${({ theme }) => theme.colors.stroke.control};
  border-bottom-color: ${({ theme }) => theme.colors.stroke.strong};
  border-radius: ${({ theme }) => theme.radius.control};
  color: ${({ theme }) => theme.colors.text.primary};
  background: ${({ theme }) => theme.colors.surface.pane};
  &::placeholder {
    color: ${({ theme }) => theme.colors.text.secondary};
  }
  &:disabled {
    color: ${({ theme }) => theme.colors.text.tertiary};
    border-bottom-color: ${({ theme }) => theme.colors.stroke.control};
  }
`;

const focusStyle = css`
  outline: none;
  border-bottom-color: ${({ theme }) => theme.colors.accent.rest};
  box-shadow: inset 0 -1px 0 ${({ theme }) => theme.colors.accent.rest};
  @media (forced-colors: active) {
    outline: 2px solid Highlight;
  }
`;

export const Input = styled.input`
  ${fieldStyle}
  &:focus {
    ${focusStyle}
  }
`;

export const Select = styled.select`
  ${fieldStyle}
  max-width: 100%;
  padding-right: 4px;
  &:focus {
    ${focusStyle}
  }
`;

export const SearchContainer = styled.div`
  ${fieldStyle}
  display: flex;
  align-items: center;
  gap: 6px;
  &:focus-within {
    ${focusStyle}
  }
  svg {
    flex-shrink: 0;
  }
`;

export const SearchInput = styled.input`
  flex: 1;
  width: 100%;
  min-width: 0;
  height: 100%;
  padding: 0;
  border: 0;
  background: transparent;
  &::placeholder {
    color: ${({ theme }) => theme.colors.text.secondary};
  }
  &:focus-visible {
    outline: none;
  }
`;
