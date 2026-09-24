// Windows 10 desktop controls: neutral surfaces, square borders, system type.
export const lightTheme = {
  colorScheme: "light",
  colors: {
    background: {
      primary: "#F3F3F3",
      secondary: "#FFFFFF",
      tertiary: "#F9F9F9",
      hover: "#F5F5F5",
      active: "#E0E0E0",
    },
    text: {
      primary: "#000000",
      secondary: "#5C5C5C",
      tertiary: "#686868",
    },
    accent: {
      primary: "#0078D4",
    },
    border: {
      light: "#E0E0E0",
      medium: "#C8C8C8",
      dark: "#A0A0A0",
    },
    status: {
      error: "#C42B1C",
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
    body: "13px",
    title: "20px",
  },
  controlHeight: { small: "28px", medium: "32px" },
  spacing: {
    xs: "4px",
    sm: "8px",
    md: "12px",
    lg: "16px",
  },
  borderRadius: {
    sm: "2px",
    lg: "2px",
  },
  shadows: {
    elevation: "0 8px 16px rgba(0, 0, 0, 0.14)",
  },
  transitions: {
    fast: "120ms ease-out",
  },
};

export const darkTheme: typeof lightTheme = {
  ...lightTheme,
  colorScheme: "dark",
  colors: {
    ...lightTheme.colors,
    background: {
      primary: "#1F1F1F",
      secondary: "#252526",
      tertiary: "#2D2D2D",
      hover: "#37373D",
      active: "#3F3F46",
    },
    text: {
      primary: "#F3F3F3",
      secondary: "#C1C1C1",
      tertiary: "#9B9B9B",
    },
    accent: {
      primary: "#0A84FF",
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

export type ThemeMode = "light" | "dark";
