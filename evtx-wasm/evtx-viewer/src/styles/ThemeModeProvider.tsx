import React, { createContext, useContext, useState } from "react";
import { ThemeProvider } from "styled-components";
import { lightTheme, darkTheme, type ThemeMode } from "./theme";

interface ThemeModeContextValue {
  mode: ThemeMode;
  toggle: () => void;
}
const ThemeModeContext = createContext<ThemeModeContextValue | null>(null);

export function useThemeMode(): ThemeModeContextValue {
  const context = useContext(ThemeModeContext);
  if (!context) throw new Error("Theme mode must be used inside ThemeModeProvider.");
  return context;
}

export function ThemeModeProvider({ children }: { children: React.ReactNode }) {
  const [mode, setMode] = useState<ThemeMode>(() => {
    try {
      return localStorage.getItem("theme-mode") === "dark" ? "dark" : "light";
    } catch {
      return "light";
    }
  });
  function toggle() {
    const next = mode === "light" ? "dark" : "light";
    setMode(next);
    try {
      localStorage.setItem("theme-mode", next);
    } catch (cause) {
      console.warn("Theme preference could not be saved.", cause);
    }
  }
  return (
    <ThemeModeContext.Provider value={{ mode, toggle }}>
      <ThemeProvider theme={mode === "dark" ? darkTheme : lightTheme}>{children}</ThemeProvider>
    </ThemeModeContext.Provider>
  );
}
