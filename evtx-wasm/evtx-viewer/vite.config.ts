import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vite.dev/config/
export default defineConfig(({ command }) => ({
  base: command === "serve" ? "/" : "/evtx/",
  plugins: [react()],
  server: {
    port: 3000,
    fs: {
      allow: [".."],
    },
  },
  build: {
    // Ensure WASM files are properly handled
    assetsInlineLimit: 0,
  },
  optimizeDeps: {
    exclude: ["./src/wasm/evtx_wasm.js"],
  },
  assetsInclude: ["**/*.wasm"],
}));
