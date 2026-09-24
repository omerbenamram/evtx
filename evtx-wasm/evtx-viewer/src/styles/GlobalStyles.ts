import { createGlobalStyle } from "styled-components";

export const GlobalStyles = createGlobalStyle`
  * {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
  }

  :root {
    color-scheme: ${({ theme }) => theme.colorScheme};
    accent-color: ${({ theme }) => theme.colors.accent.rest};
    /* Inherited; scrollbar-width is not, so it is set on every element below. */
    scrollbar-color: ${({ theme }) => theme.colors.stroke.strong} transparent;
  }

  html, body {
    height: 100%;
    overflow: hidden;
  }

  body {
    font-family: ${({ theme }) => theme.fonts.body};
    font-size: ${({ theme }) => theme.fontSize.body};
    line-height: 16px;
    color: ${({ theme }) => theme.colors.text.primary};
    background-color: ${({ theme }) => theme.colors.surface.base};
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }

  button, input, select, textarea { font: inherit; color: inherit; }

  #root {
    height: 100%;
    display: flex;
    flex-direction: column;
  }

  * {
    scrollbar-width: thin;
  }

  ::selection {
    background-color: ${({ theme }) => theme.colors.accent.rest};
    color: ${({ theme }) => theme.colors.accent.text};
  }

  :focus-visible {
    outline: 2px solid ${({ theme }) => theme.colors.focus};
    outline-offset: 1px;
  }

  :focus:not(:focus-visible) {
    outline: none;
  }

  code, pre, kbd, samp {
    font-family: ${({ theme }) => theme.fonts.mono};
    font-size: ${({ theme }) => theme.fontSize.body};
  }

  .tabular {
    font-variant-numeric: tabular-nums;
  }

  @media (prefers-reduced-motion: reduce) {
    *, *::before, *::after {
      animation-duration: 0s !important;
      animation-iteration-count: 1 !important;
      transition-duration: 0s !important;
      scroll-behavior: auto !important;
    }
  }

  @media (forced-colors: active) {
    :root { scrollbar-color: auto; }
    :focus-visible { outline-color: Highlight; }
  }
`;
