import React, { createContext, useContext, useEffect, useState } from "react";
import { ThemeProvider } from "styled-components";
import { lightTheme, darkTheme, type ThemeMode, type ThemePreference } from "./theme";

interface ThemeModeContextValue {
  /** The applied theme. */
  mode: ThemeMode;
  /** What the user picked in View > Theme; "system" follows prefers-color-scheme. */
  preference: ThemePreference;
  setPreference: (preference: ThemePreference) => void;
}
const ThemeModeContext = createContext<ThemeModeContextValue | null>(null);
const STORAGE_KEY = "theme-mode";
const darkQuery = () => window.matchMedia("(prefers-color-scheme: dark)");

export function useThemeMode(): ThemeModeContextValue {
  const context = useContext(ThemeModeContext);
  if (!context) throw new Error("Theme mode must be used inside ThemeModeProvider.");
  return context;
}

export function ThemeModeProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY);
      return saved === "light" || saved === "dark" ? saved : "system";
    } catch {
      return "system";
    }
  });
  const [systemDark, setSystemDark] = useState(() => darkQuery().matches);
  useEffect(() => {
    const query = darkQuery();
    const update = () => setSystemDark(query.matches);
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);

  function setPreference(next: ThemePreference) {
    setPreferenceState(next);
    try {
      if (next === "system") localStorage.removeItem(STORAGE_KEY);
      else localStorage.setItem(STORAGE_KEY, next);
    } catch (cause) {
      console.warn("Theme preference could not be saved.", cause);
    }
  }

  const mode: ThemeMode = preference === "system" ? (systemDark ? "dark" : "light") : preference;
  return (
    <ThemeModeContext.Provider value={{ mode, preference, setPreference }}>
      <ThemeProvider theme={mode === "dark" ? darkTheme : lightTheme}>{children}</ThemeProvider>
    </ThemeModeContext.Provider>
  );
}
