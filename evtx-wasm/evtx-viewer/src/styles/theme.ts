// Windows 11 inspired theme
export const lightTheme = {
  colors: {
    // Windows 11 color palette
    background: {
      primary: "#F3F3F3",
      secondary: "#FFFFFF",
      tertiary: "#F9F9F9",
      hover: "#F5F5F5",
      active: "#E0E0E0",
      dark: "#202020",
    },
    text: {
      primary: "#000000",
      secondary: "#5C5C5C",
      tertiary: "#8B8B8B",
      link: "#0066CC",
      white: "#FFFFFF",
    },
    accent: {
      primary: "#0078D4",
      hover: "#106EBE",
      active: "#005A9E",
      light: "#40E0D0",
    },
    border: {
      light: "#E0E0E0",
      medium: "#C8C8C8",
      dark: "#A0A0A0",
    },
    status: {
      error: "#C42B1C",
      warning: "#F7630C",
      success: "#107C10",
      info: "#0078D4",
    },
    selection: {
      background: "#E5F1FB",
      border: "#0078D4",
    },
  },
  fonts: {
    body: '"Segoe UI", -apple-system, BlinkMacSystemFont, "Roboto", "Helvetica Neue", sans-serif',
    mono: '"Cascadia Code", "Consolas", "Courier New", monospace',
  },
  fontSize: {
    caption: "12px",
    body: "14px",
    subtitle: "16px",
    title: "20px",
    header: "28px",
  },
  spacing: {
    xs: "4px",
    sm: "8px",
    md: "12px",
    lg: "16px",
    xl: "20px",
    xxl: "32px",
  },
  borderRadius: {
    sm: "4px",
    md: "6px",
    lg: "8px",
  },
  shadows: {
    sm: "0 1px 2px rgba(0, 0, 0, 0.08)",
    md: "0 2px 4px rgba(0, 0, 0, 0.08)",
    lg: "0 4px 8px rgba(0, 0, 0, 0.12)",
    elevation: "0 8px 16px rgba(0, 0, 0, 0.14)",
  },
  transitions: {
    fast: "120ms ease-out",
    normal: "200ms ease-out",
    slow: "300ms ease-out",
  },
};

// ---------------------------------------------
// Dark-mode palette – deliberately keeps the
// exact same token structure so existing styled
// components continue to work.  Only the color
// values change.  Feel free to tweak further.
// ---------------------------------------------

export const darkTheme: typeof lightTheme = {
  ...lightTheme,
  colors: {
    ...lightTheme.colors,
    background: {
      primary: "#1F1F1F",
      secondary: "#252526",
      tertiary: "#2D2D2D",
      hover: "#37373D",
      active: "#3F3F46",
      dark: "#000000",
    },
    text: {
      primary: "#F3F3F3",
      secondary: "#C1C1C1",
      tertiary: "#9B9B9B",
      link: "#3794FF",
      white: "#FFFFFF",
    },
    accent: {
      primary: "#0A84FF",
      hover: "#3391FF",
      active: "#006EDC",
      light: "#40E0D0",
    },
    border: {
      light: "#3C3C3C",
      medium: "#505050",
      dark: "#707070",
    },
    selection: {
      background: "#264F78",
      border: "#3794FF",
    },
  },
};

// Backwards-compat: keep named export `theme` pointing at light theme.
export const theme = lightTheme;

export type ThemeMode = "light" | "dark";
