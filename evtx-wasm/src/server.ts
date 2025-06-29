import { serve, file } from "bun";
import { join } from "path";

const PUBLIC_DIR = join(import.meta.dir, "..", "public");
const WASM_PKG_DIR = join(PUBLIC_DIR, "pkg");

// MIME types for proper content serving
const MIME_TYPES: Record<string, string> = {
  ".html": "text/html",
  ".js": "application/javascript",
  ".mjs": "application/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".ico": "image/x-icon",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".svg": "image/svg+xml",
};

// Get MIME type from file extension
function getMimeType(path: string): string {
  const ext = path.substring(path.lastIndexOf("."));
  return MIME_TYPES[ext] || "application/octet-stream";
}

const server = serve({
  port: process.env.PORT ? parseInt(process.env.PORT) : 3000,
  
  routes: {
    // Serve the main page
    "/": new Response(await file(join(PUBLIC_DIR, "index.html")).text(), {
      headers: {
        "Content-Type": "text/html",
        "Cache-Control": "no-cache",
      },
    }),

    // Serve WASM package files
    "/pkg/*": (req) => {
      const url = new URL(req.url);
      const filePath = join(WASM_PKG_DIR, url.pathname.slice(5)); // Remove "/pkg/" prefix
      
      try {
        const bunFile = file(filePath);
        if (!bunFile.size) throw new Error("File not found");
        
        return new Response(bunFile, {
          headers: {
            "Content-Type": getMimeType(filePath),
            "Cache-Control": "public, max-age=3600",
            // CORS headers for WASM
            "Cross-Origin-Embedder-Policy": "require-corp",
            "Cross-Origin-Opener-Policy": "same-origin",
          },
        });
      } catch {
        return new Response("Not Found", { status: 404 });
      }
    },

    // Serve static assets
    "/assets/*": (req) => {
      const url = new URL(req.url);
      const filePath = join(PUBLIC_DIR, url.pathname);
      
      try {
        const bunFile = file(filePath);
        if (!bunFile.size) throw new Error("File not found");
        
        return new Response(bunFile, {
          headers: {
            "Content-Type": getMimeType(filePath),
            "Cache-Control": "public, max-age=3600",
          },
        });
      } catch {
        return new Response("Not Found", { status: 404 });
      }
    },

    // Health check endpoint
    "/health": new Response("OK", {
      headers: { "Content-Type": "text/plain" },
    }),

    // API endpoint for server info
    "/api/info": Response.json({
      name: "EVTX WASM Explorer",
      version: "1.0.0",
      runtime: "Bun",
    }),
  },

  // Fallback for unmatched routes
  fetch(req) {
    return new Response("Not Found", { 
      status: 404,
      headers: { "Content-Type": "text/plain" },
    });
  },

  // Error handling
  error(error) {
    console.error("Server error:", error);
    return new Response(`Internal Server Error: ${error.message}`, {
      status: 500,
      headers: { "Content-Type": "text/plain" },
    });
  },
});

console.log(`🚀 EVTX WASM Explorer running at ${server.url}`);
console.log(`📁 Serving files from: ${PUBLIC_DIR}`);

// Graceful shutdown
process.on("SIGINT", () => {
  console.log("\n👋 Shutting down server...");
  server.stop();
  process.exit(0);
});