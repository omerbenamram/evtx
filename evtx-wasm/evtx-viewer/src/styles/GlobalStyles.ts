import { createGlobalStyle } from 'styled-components';
import { theme } from './theme';

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
    font-family: ${theme.fonts.body};
    font-size: ${theme.fontSize.body};
    color: ${theme.colors.text.primary};
    background-color: ${theme.colors.background.primary};
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
    background: ${theme.colors.background.primary};
    border: 1px solid ${theme.colors.border.light};
  }

  ::-webkit-scrollbar-thumb {
    background: ${theme.colors.border.medium};
    border-radius: ${theme.borderRadius.sm};
    border: 1px solid ${theme.colors.border.light};
  }

  ::-webkit-scrollbar-thumb:hover {
    background: ${theme.colors.border.dark};
  }

  /* Selection */
  ::selection {
    background-color: ${theme.colors.selection.background};
    color: ${theme.colors.text.primary};
  }

  /* Focus styles */
  :focus-visible {
    outline: 2px solid ${theme.colors.accent.primary};
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
    color: ${theme.colors.text.link};
    text-decoration: none;
    
    &:hover {
      text-decoration: underline;
    }
  }

  /* Code */
  code, pre {
    font-family: ${theme.fonts.mono};
    font-size: ${theme.fontSize.caption};
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
      background-color: ${theme.colors.text.primary};
      color: ${theme.colors.text.white};
      padding: ${theme.spacing.xs} ${theme.spacing.sm};
      border-radius: ${theme.borderRadius.sm};
      font-size: ${theme.fontSize.caption};
      white-space: nowrap;
      opacity: 0;
      visibility: hidden;
      transition: opacity ${theme.transitions.fast};
      margin-bottom: ${theme.spacing.xs};
    }
    
    &:hover::after {
      opacity: 1;
      visibility: visible;
    }
  }
`;