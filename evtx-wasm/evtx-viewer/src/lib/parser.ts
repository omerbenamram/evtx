// EVTX Parser interface - wraps the WASM module
import type { EvtxFileInfo, ParseResult, EvtxRecord } from "./types";

// Minimal runtime shape of the WASM parser instance. We only include the
// methods we actually call from the TypeScript side.
interface WasmParserInstance {
  parse_all(): unknown;
  parse_with_limit(limit?: number): unknown;
  parse_chunk(chunkIndex: number): unknown;
  get_record_by_id(recordId: number): unknown;
}

export interface IEvtxParser {
  parseFile(file: File): Promise<EvtxFileInfo>;
  parseAllRecords(): Promise<ParseResult>;
  parseChunk(chunkIndex: number): Promise<ParseResult>;
  parseWithLimit(limit: number): Promise<ParseResult>;
  getRecordById(recordId: number): Promise<EvtxRecord | null>;
  exportRecords(records: EvtxRecord[], format: "json" | "xml"): string;
}

export class EvtxParser implements IEvtxParser {
  private wasmParser: WasmParserInstance | null = null;
  private fileData: Uint8Array | null = null;

  /**
   * Normalise the raw `ParseResult` returned from the WASM bindings to the
   * camelCase `ParseResult` expected by the rest of the TypeScript codebase.
   */
  private normaliseParseResult(raw: unknown): ParseResult {
    const obj = raw as Record<string, unknown>;

    // Helper to deeply convert Map instances produced by `serde_wasm_bindgen`
    // into plain JavaScript objects so React/TS accessors work as expected.
    const mapToObject = (input: unknown): unknown => {
      if (input instanceof Map) {
        const out: Record<string, unknown> = {};
        input.forEach((v, k) => {
          out[k as string] = mapToObject(v);
        });
        return out;
      }
      if (Array.isArray(input)) {
        return input.map((el) => mapToObject(el));
      }
      return input;
    };

    // Ensure `records` is an array of objects (may arrive as JSON strings).
    const records: EvtxRecord[] = ((obj.records as unknown[]) ?? []).map(
      (r: unknown) => {
        if (typeof r === "string") {
          try {
            return JSON.parse(r) as EvtxRecord;
          } catch {
            // Fallback – an unparsable string. Return a placeholder to avoid crashing.
            return { Event: { System: {} } } as unknown as EvtxRecord;
          }
        }
        // Convert Map → object recursively if needed
        const transformed = mapToObject(r);
        return transformed as EvtxRecord;
      }
    );

    return {
      records,
      totalRecords:
        (obj.total_records as number | undefined) ??
        (obj.totalRecords as number | undefined) ??
        records.length,
      errors: (obj.errors as string[] | undefined) ?? [],
    };
  }

  async parseFile(file: File): Promise<EvtxFileInfo> {
    const arrayBuffer = await file.arrayBuffer();
    this.fileData = new Uint8Array(arrayBuffer);

    // Dynamically import WASM module
    const { quick_file_info, EvtxWasmParser } = await import(
      "../wasm/evtx_wasm.js"
    );

    // Get file info
    const fileInfo = await quick_file_info(this.fileData);

    // Create parser instance – we cast it to the minimal interface we defined
    // above. This avoids introducing `any` while still acknowledging the
    // dynamic nature of the WASM import.
    this.wasmParser = new EvtxWasmParser(
      this.fileData
    ) as unknown as WasmParserInstance;

    return {
      fileName: file.name,
      fileSize: file.size,
      totalChunks: fileInfo.total_chunks as number,
      nextRecordId: fileInfo.next_record_id as string,
      isDirty: fileInfo.is_dirty as boolean,
      isFull: fileInfo.is_full as boolean,
      chunks: (fileInfo.chunks as unknown[]).map((c: unknown) => {
        const chunkObj = c as Record<string, unknown>;
        return {
          chunkNumber: chunkObj.chunk_number as number,
          recordCount: chunkObj.record_count as string,
          firstRecordId: chunkObj.first_record_id as string,
          lastRecordId: chunkObj.last_record_id as string,
        };
      }),
    } as EvtxFileInfo;
  }

  async parseAllRecords(): Promise<ParseResult> {
    if (!this.wasmParser) {
      throw new Error("No file loaded");
    }

    const raw = await this.wasmParser.parse_all();
    return this.normaliseParseResult(raw);
  }

  async parseChunk(chunkIndex: number): Promise<ParseResult> {
    if (!this.wasmParser) {
      throw new Error("No file loaded");
    }

    const raw = await this.wasmParser.parse_chunk(chunkIndex);
    return this.normaliseParseResult(raw);
  }

  async parseWithLimit(limit: number): Promise<ParseResult> {
    if (!this.wasmParser) {
      throw new Error("No file loaded");
    }

    const raw = await this.wasmParser.parse_with_limit(limit);
    return this.normaliseParseResult(raw);
  }

  async getRecordById(recordId: number): Promise<EvtxRecord | null> {
    if (!this.wasmParser) {
      throw new Error("No file loaded");
    }

    try {
      return (await this.wasmParser.get_record_by_id(
        recordId
      )) as EvtxRecord | null;
    } catch (error) {
      console.error(`Failed to get record ${recordId}:`, error);
      return null;
    }
  }

  exportRecords(records: EvtxRecord[], format: "json" | "xml"): string {
    if (format === "json") {
      return JSON.stringify(records, null, 2);
    }

    // For XML export, we would need to implement XML serialization
    // For now, we'll use the JSON representation
    // In a real implementation, we'd call the WASM parser's XML export
    throw new Error("XML export not yet implemented in browser");
  }
}
