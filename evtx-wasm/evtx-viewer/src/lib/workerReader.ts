import type { ParserCommand, ParserRequest, ParserResponse } from "./workerProtocol";
import { EvtxChunkError, parserResponseSchema } from "./workerProtocol";

export class EvtxWorkerReader {
  private readonly worker = new Worker(new URL("./evtx.worker.ts", import.meta.url), {
    type: "module",
  });
  private sequence = 0;
  private closedReason: Error | null = null;
  private readonly pending = new Map<
    number,
    {
      resolve: (value: ParserResponse) => void;
      reject: (reason: Error) => void;
    }
  >();

  constructor(private readonly signal: AbortSignal) {
    this.worker.addEventListener("message", (event: MessageEvent) => {
      const parsed = parserResponseSchema.safeParse(event.data);
      if (!parsed.success) {
        this.close(
          new Error("The parser worker returned an invalid response.", { cause: parsed.error }),
        );
        return;
      }
      const data = parsed.data;
      const request = this.pending.get(data.id);
      this.pending.delete(data.id);
      if (data.type === "error") {
        const error = new Error(data.error);
        request?.reject(error);
        this.close(error);
      } else if (data.type === "chunk-error") request?.reject(new EvtxChunkError(data.error));
      else request?.resolve(data);
    });
    this.worker.addEventListener("error", (event) =>
      this.close(new Error(event.message || "The parser worker stopped unexpectedly.")),
    );
    this.worker.addEventListener("messageerror", () =>
      this.close(new Error("The parser worker returned an unreadable response.")),
    );
    signal.addEventListener("abort", this.abort, { once: true });
    if (signal.aborted) this.abort();
  }

  private readonly abort = () => this.close(new DOMException("Import cancelled", "AbortError"));

  private request(data: ParserCommand): Promise<ParserResponse> {
    if (this.closedReason) return Promise.reject(this.closedReason);
    const id = ++this.sequence;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      try {
        const request: ParserRequest = { ...data, id };
        this.worker.postMessage(request, []);
      } catch (error) {
        this.close(error instanceof Error ? error : new Error(String(error)));
      }
    });
  }

  async open(file: File) {
    const result = await this.request({ type: "open", file });
    if (result.type !== "open") throw new Error("Unexpected parser response");
    return result;
  }

  async getArrowIPCChunk(index: number) {
    const result = await this.request({ type: "chunk", index });
    if (result.type !== "chunk") throw new Error("Unexpected parser response");
    return result;
  }

  close(reason: Error = new DOMException("Parser closed", "AbortError")) {
    if (this.closedReason) return;
    this.closedReason = reason;
    this.worker.terminate();
    this.signal.removeEventListener("abort", this.abort);
    for (const request of this.pending.values()) request.reject(reason);
    this.pending.clear();
  }
}
