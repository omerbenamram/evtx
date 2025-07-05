// @vitest-environment node
import { describe, it, expect, beforeAll } from "vitest";

// Polyfill - attaches IDB* globals automatically
import "fake-indexeddb/auto";

import EvtxStorage from "../storage";

function makeFakeFile(size = 128 * 1024, name = "sample.evtx"): File {
  const content = new Uint8Array(size);
  // Fill with deterministic data
  for (let i = 0; i < size; i++) content[i] = i % 256;
  return new File([content], name);
}

describe("EvtxStorage", () => {
  let storage: EvtxStorage;

  beforeAll(async () => {
    storage = await EvtxStorage.getInstance();
  });

  it("saves file and retrieves metadata", async () => {
    const file = makeFakeFile();
    const fileId = await storage.saveFile(file, 2);
    const files = await storage.listFiles();
    const meta = files.find((f) => f.fileId === fileId);
    expect(meta).toBeDefined();
    expect(meta!.fileSize).toBe(file.size);
  });

  it("gets correct chunk slice", async () => {
    const file = makeFakeFile();
    const fileId = await storage.saveFile(file, 2);
    const chunk = await storage.getChunk(fileId, 0);
    expect(chunk.byteLength).toBe(0x10000);
  });
});
