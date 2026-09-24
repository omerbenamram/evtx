// @vitest-environment node
import * as indexedDB from "fake-indexeddb";
import { beforeAll, expect, it, vi } from "vitest";
import EvtxStorage from "../storage";

beforeAll(() => {
  for (const [name, value] of Object.entries(indexedDB)) {
    if (name === "indexedDB" || name.startsWith("IDB")) vi.stubGlobal(name, value);
  }
});
it("reopens and deletes saved log bytes and shares concurrent initialization", async () => {
  const [storage, concurrent] = await Promise.all([
    EvtxStorage.getInstance(),
    EvtxStorage.getInstance(),
  ]);
  expect(storage).toBe(concurrent);
  const file = new File([new Uint8Array([1, 2, 3])], "sample.evtx");
  const id = await storage.saveFile(file, 2);
  expect(await storage.listFiles()).toEqual([
    expect.objectContaining({ fileId: id, fileSize: 3, chunkCount: 2 }),
  ]);
  const stored = await storage.getFile(id);
  expect(new Uint8Array(await stored.blob.arrayBuffer())).toEqual(new Uint8Array([1, 2, 3]));
  await storage.deleteFile(id);
  expect(await storage.listFiles()).toEqual([]);
  await expect(storage.getFile(id)).rejects.toThrow("The saved log was not found");
});
