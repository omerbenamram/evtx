#!/usr/bin/env bun
import { $ } from "bun";
import { join } from "path";
import { existsSync, mkdirSync } from "fs";

console.log("🔨 Building EVTX WASM Explorer...\n");

// Create necessary directories
const dirs = ["public/pkg", "public/assets", "dist"];
for (const dir of dirs) {
  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });
    console.log(`📁 Created directory: ${dir}`);
  }
}

try {
  // Step 1: Build WASM module
  console.log("📦 Building WASM module...");
  await $`wasm-pack build --target web --out-dir public/pkg`;
  console.log("✅ WASM module built successfully\n");

  // Step 2: Transpile TypeScript app
  console.log("🔄 Transpiling TypeScript app...");
  await Bun.build({
    entrypoints: ["./src/app.ts"],
    outdir: "./public/assets",
    target: "browser",
    format: "esm",
    minify: process.env.NODE_ENV === "production",
    sourcemap: process.env.NODE_ENV !== "production" ? "external" : "none",
    external: ["/pkg/evtx_wasm.js"],
  });
  console.log("✅ TypeScript app transpiled successfully\n");

  // Step 3: Build server for production
  if (process.env.NODE_ENV === "production") {
    console.log("🚀 Building server for production...");
    await Bun.build({
      entrypoints: ["./src/server.ts"],
      outdir: "./dist",
      target: "bun",
      minify: true,
    });
    console.log("✅ Server built successfully\n");
  }

  console.log("🎉 Build completed successfully!");
  console.log("\nTo run the development server:");
  console.log("  bun run dev");
  console.log("\nTo run the production server:");
  console.log("  NODE_ENV=production bun run start");

} catch (error) {
  console.error("❌ Build failed:", error);
  process.exit(1);
}