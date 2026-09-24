import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App.tsx";
import { ThemeModeProvider } from "./styles/ThemeModeProvider";
import { GlobalProvider } from "./state/store";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeModeProvider>
      <GlobalProvider>
        <App />
      </GlobalProvider>
    </ThemeModeProvider>
  </StrictMode>,
);
