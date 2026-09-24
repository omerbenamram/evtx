import { expect, it } from "vitest";
import { startFullIngest } from "../fullIngest";
import { EvtxChunkError, type ParserResponse } from "../workerProtocol";
import { deferred } from "./database";

function chunk(index: number): Extract<ParserResponse, { type: "chunk" }> {
  return { id: index, type: "chunk", buffer: new Uint8Array([index]).buffer, rows: 2, errors: [] };
}

it("awaits each batch write before requesting or publishing another chunk", async () => {
  const entered = deferred();
  const release = deferred();
  const requested: number[] = [];
  const written: number[][] = [];
  const updates: number[] = [];
  const reader = {
    getArrowIPCChunk: async (index: number) => {
      requested.push(index);
      return chunk(index);
    },
  };
  const importing = startFullIngest(
    reader,
    3,
    new AbortController().signal,
    (_progress, records) => {
      updates.push(records);
    },
    () => {},
    async (bytes) => {
      written.push(Array.from(new Uint8Array(bytes)));
      entered.resolve();
      await release.promise;
    },
  );
  await entered.promise;
  expect(requested).toEqual([0]);
  expect(updates).toEqual([]);
  release.resolve();
  await expect(importing).resolves.toBe(6);
  expect(written).toEqual([[0], [1], [2]]);
  expect(updates.at(-1)).toBe(6);
});

it.each(["parsing", "inserting"] as const)(
  "stops after cancellation during %s without writing another batch",
  async (stage) => {
    const entered = deferred();
    const release = deferred();
    const controller = new AbortController();
    let requested = 0;
    let written = 0;
    let published = 0;
    const reader = {
      getArrowIPCChunk: async (index: number) => {
        requested++;
        if (stage === "parsing") {
          entered.resolve();
          await release.promise;
        }
        return chunk(index);
      },
    };
    const importing = startFullIngest(
      reader,
      3,
      controller.signal,
      () => {
        published++;
      },
      () => {},
      async () => {
        written++;
        entered.resolve();
        await release.promise;
      },
    );
    await entered.promise;
    controller.abort();
    release.resolve();
    await expect(importing).rejects.toMatchObject({ name: "AbortError" });
    expect(requested).toBe(1);
    expect(written).toBe(stage === "inserting" ? 1 : 0);
    expect(published).toBe(0);
  },
);

it("propagates failed writes without publishing or skipping the failed batch", async () => {
  let requested = 0;
  const updates: number[] = [];
  const warnings: string[] = [];
  const reader = {
    getArrowIPCChunk: async (index: number) => {
      requested++;
      return chunk(index);
    },
  };
  await expect(
    startFullIngest(
      reader,
      3,
      new AbortController().signal,
      (_progress, records) => {
        updates.push(records);
      },
      (warning) => {
        warnings.push(warning);
      },
      async () => {
        throw new Error("database is full");
      },
    ),
  ).rejects.toThrow("database is full");
  expect(requested).toBe(1);
  expect(updates).toEqual([]);
  expect(warnings).toEqual([]);
});

it("continues after a malformed chunk and reports recoverable record warnings", async () => {
  let written = 0;
  const warnings: string[] = [];
  const reader = {
    getArrowIPCChunk: async (index: number) => {
      if (index === 0) throw new EvtxChunkError("bad chunk magic");
      return { ...chunk(index), errors: ["one damaged record skipped"] };
    },
  };
  await expect(
    startFullIngest(
      reader,
      2,
      new AbortController().signal,
      () => {},
      (warning) => {
        warnings.push(warning);
      },
      async () => {
        written++;
      },
    ),
  ).resolves.toBe(2);
  expect(written).toBe(1);
  expect(warnings).toEqual(["Chunk 1: bad chunk magic", "Chunk 2: one damaged record skipped"]);
});

it("propagates a fatal reader failure instead of reporting every remaining chunk as corrupt", async () => {
  let requested = 0;
  let written = 0;
  const warnings: string[] = [];
  const reader = {
    getArrowIPCChunk: async () => {
      requested++;
      throw new Error("WASM crashed");
    },
  };
  await expect(
    startFullIngest(
      reader,
      10,
      new AbortController().signal,
      () => {},
      (warning) => {
        warnings.push(warning);
      },
      async () => {
        written++;
      },
    ),
  ).rejects.toThrow("WASM crashed");
  expect(requested).toBe(1);
  expect(written).toBe(0);
  expect(warnings).toEqual([]);
});
