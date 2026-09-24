import init, { EvtxWasmParser } from "../wasm/evtx_wasm.js";
import { errorMessage, evtxFileInfoSchema } from "./types";
import type { ParserRequest, ParserResponse } from "./workerProtocol";

let parser: EvtxWasmParser | undefined;
const reply = (response: ParserResponse, transfer: Transferable[] = []) =>
  self.postMessage(response, { transfer });

self.addEventListener("message", async ({ data }: MessageEvent<ParserRequest>) => {
  try {
    if (data.type === "open") {
      await init();
      parser?.free();
      parser = new EvtxWasmParser(new Uint8Array(await data.file.arrayBuffer()));
      const info = evtxFileInfoSchema.parse({
        totalChunks: parser.total_chunks,
        fileName: data.file.name,
      });
      reply({ id: data.id, type: "open", info });
    } else {
      if (!parser) throw new Error("No file is open");
      let chunk;
      try {
        chunk = parser.chunk_arrow_ipc(data.index);
      } catch (error) {
        // A WASM trap invalidates the parser; a returned chunk error is recoverable.
        if (error instanceof WebAssembly.RuntimeError) throw error;
        reply({ id: data.id, type: "chunk-error", error: errorMessage(error) });
        return;
      }
      try {
        // The Rust getter already copies out of WASM memory; transfer that buffer.
        const buffer = chunk.ipc.buffer;
        if (!(buffer instanceof ArrayBuffer))
          throw new Error("The parser returned an invalid Arrow buffer.");
        reply({ id: data.id, type: "chunk", buffer, rows: chunk.rows, errors: chunk.errors }, [
          buffer,
        ]);
      } finally {
        chunk.free();
      }
    }
  } catch (error) {
    reply({ id: data.id, type: "error", error: errorMessage(error) });
  }
});
