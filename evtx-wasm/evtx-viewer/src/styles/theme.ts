// Classic Event Viewer layout drawn with Windows 11 (WinUI) colors and metrics. See DESIGN.md.
export const lightTheme = {
  colorScheme: "light",
  colors: {
    surface: {
      base: "#F3F3F3", // behind menu bar, toolbar, status bar
      pane: "#FFFFFF", // tree, grid, facets, details, flyouts
    },
    stroke: {
      divider: "#E5E5E5", // pane separators, flyout border
      control: "#D1D1D1", // button and field border
      strong: "#8A8A8A", // field bottom edge, scrollbar thumb
    },
    text: {
      primary: "#1A1A1A",
      secondary: "#5D5D5D",
      tertiary: "#8A8A8A",
    },
    accent: {
      rest: "#005FB8",
      hover: "#1A6DBF",
      pressed: "#3380C7",
      text: "#FFFFFF", // text on accent fill
    },
    band: {
      background: "#6E6E6E",
      text: "#FFFFFF",
    },
    severity: {
      error: "#C42B1C", // critical and error
      warning: "#9D5D00",
      verbose: "#8A8A8A",
    },
    fill: {
      hover: "rgba(0, 0, 0, 0.04)",
      pressed: "rgba(0, 0, 0, 0.02)",
      selected: "rgba(0, 95, 184, 0.12)",
    },
    focus: "#005FB8",
  },
  fonts: {
    body: '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif',
    mono: '"Cascadia Mono", Consolas, monospace',
  },
  fontSize: {
    body: "12px",
    secondary: "11px",
    title: "20px",
  },
  size: {
    toolbarControl: "24px",
    control: "28px",
    row: "22px",
    header: "24px",
  },
  spacing: {
    xs: "4px",
    sm: "8px",
    md: "12px",
    lg: "16px",
  },
  radius: {
    control: "4px",
    flyout: "8px",
  },
  shadow: {
    flyout: "0 8px 16px rgba(0, 0, 0, 0.14)",
  },
  motion: {
    fast: "120ms cubic-bezier(0, 0, 0, 1)",
  },
};

export const darkTheme: typeof lightTheme = {
  ...lightTheme,
  colorScheme: "dark",
  colors: {
    surface: { base: "#202020", pane: "#2B2B2B" },
    stroke: { divider: "#1D1D1D", control: "#454545", strong: "#9A9A9A" },
    text: { primary: "#FFFFFF", secondary: "#C5C5C5", tertiary: "#9A9A9A" },
    accent: { rest: "#60CDFF", hover: "#5AB8E6", pressed: "#52A6CF", text: "#000000" },
    band: { background: "#3A3A3A", text: "#FFFFFF" },
    severity: { error: "#FF99A4", warning: "#FCE100", verbose: "#9A9A9A" },
    fill: {
      hover: "rgba(255, 255, 255, 0.06)",
      pressed: "rgba(255, 255, 255, 0.04)",
      selected: "rgba(96, 205, 255, 0.18)",
    },
    focus: "#60CDFF",
  },
};

export type ThemeMode = "light" | "dark";
export type ThemePreference = ThemeMode | "system";
