import { createGlobalStyle } from "styled-components";

export const GlobalStyles = createGlobalStyle`
  * {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
  }

  html, body {
    height: 100%;
    overflow: hidden;
  }

  body {
    font-family: ${({ theme }) => theme.fonts.body};
    font-size: ${({ theme }) => theme.fontSize.body};
    color: ${({ theme }) => theme.colors.text.primary};
    background-color: ${({ theme }) => theme.colors.background.primary};
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }

  #root {
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  /* Windows-style scrollbar */
  ::-webkit-scrollbar {
    width: 12px;
    height: 12px;
  }

  ::-webkit-scrollbar-track {
    background: ${({ theme }) => theme.colors.background.primary};
    border: 1px solid ${({ theme }) => theme.colors.border.light};
  }

  ::-webkit-scrollbar-thumb {
    background: ${({ theme }) => theme.colors.border.medium};
    border-radius: ${({ theme }) => theme.borderRadius.sm};
    border: 1px solid ${({ theme }) => theme.colors.border.light};
  }

  ::-webkit-scrollbar-thumb:hover {
    background: ${({ theme }) => theme.colors.border.dark};
  }

  /* Selection */
  ::selection {
    background-color: ${({ theme }) => theme.colors.selection.background};
    color: ${({ theme }) => theme.colors.text.primary};
  }

  /* Focus styles */
  :focus-visible {
    outline: 2px solid ${({ theme }) => theme.colors.accent.primary};
    outline-offset: 2px;
  }

  /* Disable focus outline for mouse users */
  :focus:not(:focus-visible) {
    outline: none;
  }

  /* Tables */
  table {
    border-collapse: collapse;
    width: 100%;
  }

  /* Links */
  a {
    color: ${({ theme }) => theme.colors.text.link};
    text-decoration: none;

    &:hover {
      text-decoration: underline;
    }
  }

  /* Code */
  code, pre {
    font-family: ${({ theme }) => theme.fonts.mono};
    font-size: ${({ theme }) => theme.fontSize.caption};
  }

  /* Tooltips */
  [data-tooltip] {
    position: relative;

    &::after {
      content: attr(data-tooltip);
      position: absolute;
      bottom: 100%;
      left: 50%;
      transform: translateX(-50%);
      background-color: ${({ theme }) => theme.colors.text.primary};
      color: ${({ theme }) => theme.colors.text.white};
      padding: ${({ theme }) => theme.spacing.xs} ${({ theme }) =>
  theme.spacing.sm};
      border-radius: ${({ theme }) => theme.borderRadius.sm};
      font-size: ${({ theme }) => theme.fontSize.caption};
      white-space: nowrap;
      opacity: 0;
      visibility: hidden;
      transition: opacity ${({ theme }) => theme.transitions.fast};
      margin-bottom: ${({ theme }) => theme.spacing.xs};
    }

    &:hover::after {
      opacity: 1;
      visibility: visible;
    }
  }
`;
