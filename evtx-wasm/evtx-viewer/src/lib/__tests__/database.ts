import { createRequire } from "node:module";
import { createDuckDB, NODE_RUNTIME, VoidLogger } from "@duckdb/duckdb-wasm/blocking";

export async function openTestDatabase() {
  const require = createRequire(import.meta.url);
  const database = await createDuckDB(
    {
      mvp: {
        mainModule: require.resolve("@duckdb/duckdb-wasm/dist/duckdb-mvp.wasm"),
        mainWorker: require.resolve("@duckdb/duckdb-wasm/dist/duckdb-node-mvp.worker.cjs"),
      },
      eh: {
        mainModule: require.resolve("@duckdb/duckdb-wasm/dist/duckdb-eh.wasm"),
        mainWorker: require.resolve("@duckdb/duckdb-wasm/dist/duckdb-node-eh.worker.cjs"),
      },
    },
    new VoidLogger(),
    NODE_RUNTIME,
  );
  await database.instantiate();
  return database.connect();
}

export function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
