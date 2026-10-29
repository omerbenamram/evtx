import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { createDuckDB, NODE_RUNTIME, VoidLogger } from "@duckdb/duckdb-wasm/blocking";

type TestConnection = ReturnType<Awaited<ReturnType<typeof createDuckDB>>["connect"]>;

const WASM_MAGIC = [0x00, 0x61, 0x73, 0x6d];
const DOWNLOAD_TIMEOUT_MS = 30_000;
// Pass to the beforeAll that calls openTestDatabase: a cold cache downloads the extension.
export const OPEN_TIMEOUT_MS = DOWNLOAD_TIMEOUT_MS + 10_000;

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
  const connection = database.connect();
  try {
    await cacheJsonExtension(connection);
  } catch (error) {
    connection.close();
    throw error;
  }
  return connection;
}

// DuckDB autoloads its json extension on first use. In Node, the blocking build fetches it
// on a helper thread while the main thread waits in Atomics.wait with no timeout, so an
// unreachable extensions.duckdb.org (an offline machine, a proxy) hangs the run forever.
// Downloading it first into the cache that loader reads turns that into a fast, clear error.
// The cache layout is duckdb-wasm's own (its Node loader in duckdb-node-blocking.cjs), so
// check it still matches when bumping the pinned @duckdb/duckdb-wasm version.
async function cacheJsonExtension(connection: TestConnection) {
  const [{ version, platform }] = connection
    .query(
      "SELECT library_version AS version, (SELECT platform FROM pragma_platform()) AS platform FROM pragma_version()",
    )
    .toArray();
  const path = `${version}/${platform}/json.duckdb_extension.wasm`;
  const file = join(homedir(), ".duckdb/extensions/extensions.duckdb.org", path);
  if (isWasm(readCached(file))) return;
  const url = `https://extensions.duckdb.org/${path}`;
  let bytes: Uint8Array;
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MS) });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    bytes = new Uint8Array(await response.arrayBuffer());
    if (!isWasm(bytes)) throw new Error("response is not a wasm module");
  } catch (cause) {
    throw new Error(
      `Could not download DuckDB's json extension from ${url}. The DuckDB tests need network ` +
        `access to extensions.duckdb.org (behind a proxy, set NODE_USE_ENV_PROXY=1), or the file ` +
        `already at ${file}.`,
      { cause },
    );
  }
  mkdirSync(dirname(file), { recursive: true });
  // Test files run in parallel, so publish the file atomically.
  const partial = `${file}.${process.pid}.partial`;
  writeFileSync(partial, bytes);
  renameSync(partial, file);
}

function readCached(file: string) {
  try {
    return new Uint8Array(readFileSync(file));
  } catch {
    return undefined;
  }
}

// A file a killed run left half-written, or an error page, would make DuckDB fail to load it.
function isWasm(bytes: Uint8Array | undefined) {
  return (
    bytes !== undefined && bytes.length > 8 && WASM_MAGIC.every((byte, i) => bytes[i] === byte)
  );
}

export function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
