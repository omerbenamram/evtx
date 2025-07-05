import { openDB, type DBSchema, type IDBPDatabase } from "idb";

// -----------------------------
// DB Schema
// -----------------------------
interface EvtxDB extends DBSchema {
  files: {
    // primary key is fileId (hash of name+size)
    key: string; // fileId
    value: {
      fileId: string; // redundant but convenient for reads
      fileName: string;
      fileSize: number;
      lastOpened: number; // epoch ms
      pinned: boolean;
      chunkCount: number;
      totalRecords?: number;
      // We keep the full file as a single Blob for now – slicing gives us chunk data
      blob: Blob;
      bucketCounts?: import("./types").BucketCounts;
    };
  };
}

// -----------------------------
// Helper
// -----------------------------
class EvtxStorage {
  private static instance: EvtxStorage;
  private db!: IDBPDatabase<EvtxDB>;

  private constructor() {}

  static async getInstance(): Promise<EvtxStorage> {
    if (!EvtxStorage.instance) {
      const inst = new EvtxStorage();
      await inst.init();
      EvtxStorage.instance = inst;
    }
    return EvtxStorage.instance;
  }

  private async init() {
    this.db = await openDB<EvtxDB>("evtx-viewer", 1, {
      upgrade(db: IDBPDatabase<EvtxDB>) {
        db.createObjectStore("files", {
          keyPath: "fileId",
        });
      },
    });
  }

  // Derive deterministic ID → `${name}_${size}_${mtime}`
  async deriveFileId(file: File): Promise<string> {
    // We can’t get mtime directly from the browser File object (webkitRelativePath aside).
    // So we hash name+size and current date if duplicates matter.
    return `${file.name}_${file.size}`;
  }

  async saveFile(
    file: File,
    chunkCount: number,
    totalRecords?: number
  ): Promise<string> {
    const fileId = await this.deriveFileId(file);
    const tx = this.db.transaction("files", "readwrite");
    await tx.store.put({
      fileId,
      fileName: file.name,
      fileSize: file.size,
      lastOpened: Date.now(),
      pinned: false,
      chunkCount,
      totalRecords,
      blob: file,
      bucketCounts: undefined,
    });
    await tx.done;
    return fileId;
  }

  async touchFile(fileId: string) {
    const rec = await this.db.get("files", fileId);
    if (rec) {
      rec.lastOpened = Date.now();
      await this.db.put("files", rec);
    }
  }

  async listFiles(): Promise<EvtxDB["files"]["value"][]> {
    return await this.db.getAll("files");
  }

  async deleteFile(fileId: string) {
    await this.db.delete("files", fileId);
  }

  /** Retrieve both metadata and blob for a stored file. */
  async getFile(
    fileId: string
  ): Promise<{ meta: EvtxDB["files"]["value"]; blob: Blob }> {
    const rec = await this.db.get("files", fileId);
    if (!rec) throw new Error("file not found");
    return { meta: rec, blob: rec.blob };
  }

  async setPinned(fileId: string, pinned: boolean) {
    const rec = await this.db.get("files", fileId);
    if (rec) {
      rec.pinned = pinned;
      await this.db.put("files", rec);
    }
  }

  // -----------------------------
  // Bucket counts helpers
  // -----------------------------

  /** Store pre-computed bucket counts for the given file */
  async saveBucketCounts(
    fileId: string,
    buckets: import("./types").BucketCounts
  ) {
    const rec = await this.db.get("files", fileId);
    if (rec) {
      rec.bucketCounts = buckets;
      await this.db.put("files", rec);
    }
  }

  /** Retrieve bucket counts if they were previously computed */
  async getBucketCounts(
    fileId: string
  ): Promise<import("./types").BucketCounts | undefined> {
    const rec = await this.db.get("files", fileId);
    return rec?.bucketCounts;
  }

  // Return the Blob slice that corresponds to the given chunk.
  async getChunk(
    fileId: string,
    chunkIndex: number,
    chunkSize = 0x10000 /* 64 KiB */
  ): Promise<ArrayBuffer> {
    const rec = await this.db.get("files", fileId);
    if (!rec) throw new Error("file not found");
    const start = chunkIndex * chunkSize;
    const end = Math.min(start + chunkSize, rec.fileSize);
    return rec.blob.slice(start, end).arrayBuffer();
  }
}

export default EvtxStorage;
