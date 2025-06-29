# EVTX WASM Parser Explorer

A modern web-based EVTX (Windows Event Log) file parser and explorer built with Rust, WebAssembly, TypeScript, and Bun.

## Features

- 🚀 **Fast Performance**: Native performance with WebAssembly
- 📁 **Drag-and-drop**: Easy file loading with drag-and-drop interface
- 🔍 **Search**: Real-time search through parsed records
- 📊 **Chunk Navigation**: Browse through EVTX chunks efficiently
- 💾 **Export**: Export parsed records as JSON
- 🎨 **Modern UI**: Clean, responsive interface with dark mode support
- 🔒 **Privacy**: All processing happens in your browser
- 🔥 **Hot Reload**: Development mode with automatic reloading

## Tech Stack

- **Backend**: Bun native server with TypeScript
- **Frontend**: TypeScript with modern ES modules
- **Parser**: Rust compiled to WebAssembly
- **Styling**: Modern CSS with CSS variables
- **Build**: Bun for blazing fast builds

## Prerequisites

- [Bun](https://bun.sh) (latest version)
- [Rust](https://rustup.rs/) with `wasm-pack`
- [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/)

## Installation

1. Clone the repository
2. Install dependencies:
   ```bash
   bun install
   ```

3. Install wasm-pack if you haven't already:
   ```bash
   cargo install wasm-pack
   ```

## Development

Run the development server with hot reload:

```bash
bun run dev
```

The server will start at `http://localhost:3000` with automatic reloading on file changes.

## Building

Build for development:
```bash
bun run build
```

Build for production:
```bash
bun run build:prod
```

## Production

Start the production server:
```bash
NODE_ENV=production bun run start
```

## Usage

1. **Load File**: Drag an EVTX file onto the drop zone or click to browse
2. **View File Info**: See file metadata including chunk count and status
3. **Browse Chunks**: Click on any chunk to select it
4. **Parse Records**: 
   - "Parse All Records" - Parse up to 1000 records from all chunks
   - "Parse Selected Chunk" - Parse only the selected chunk
5. **Search**: Use the search box to filter records in real-time
6. **View Details**: Click on any record to see the full JSON structure
7. **Export**: Click "Export JSON" to download the parsed records

## Architecture

```
evtx-wasm/
├── src/
│   ├── lib.rs          # Rust WASM bindings
│   ├── server.ts       # Bun server with routing
│   └── app.ts          # Frontend TypeScript application
├── public/
│   ├── index.html      # Main HTML file
│   ├── assets/         # CSS and compiled JS
│   └── pkg/            # WASM build output
├── Cargo.toml          # Rust dependencies
├── package.json        # Node dependencies
└── tsconfig.json       # TypeScript configuration
```

## Performance

- **Zero-copy file transfers** using Bun's native file serving
- **Streaming support** for large files
- **Efficient chunk-based parsing** to handle large EVTX files
- **Client-side processing** - files never leave your machine
- **Optimized WASM** with size optimization (`opt-level = "z"`)

## Configuration

Server configuration via environment variables:
- `PORT` - Server port (default: 3000)
- `NODE_ENV` - Environment mode (development/production)

## Scripts

- `bun run dev` - Start development server with hot reload
- `bun run build` - Build for development
- `bun run build:prod` - Build for production
- `bun run start` - Start production server
- `bun run clean` - Clean build artifacts

## Browser Support

- Chrome/Edge 88+
- Firefox 89+
- Safari 15+
- Any browser with WebAssembly and ES modules support

## Limitations

- Limited to parsing 1000 records at once for UI performance
- Large EVTX files may take time to load initially
- Browser memory constraints apply to very large files

## License

MIT/Apache-2.0 (same as the parent evtx crate)