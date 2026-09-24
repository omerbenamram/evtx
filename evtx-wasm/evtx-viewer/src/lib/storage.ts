import { openDB, type DBSchema, type IDBPDatabase } from "idb";

interface StoredLog {
  fileId: string;
  fileName: string;
  fileSize: number;
  lastOpened: number;
  chunkCount: number;
  blob: Blob;
}
interface EvtxDB extends DBSchema {
  files: { key: string; value: StoredLog };
}

export const fileIdFor = (file: File) => `${file.name}_${file.size}`;

class EvtxStorage {
  private static opening: Promise<EvtxStorage> | undefined;
  private constructor(private readonly db: IDBPDatabase<EvtxDB>) {}

  static getInstance(): Promise<EvtxStorage> {
    EvtxStorage.opening ??= openDB<EvtxDB>("evtx-viewer", 1, {
      upgrade(db) {
        db.createObjectStore("files", { keyPath: "fileId" });
      },
    })
      .then((db) => new EvtxStorage(db))
      .catch((cause: unknown) => {
        EvtxStorage.opening = undefined;
        throw cause;
      });
    return EvtxStorage.opening;
  }

  async saveFile(file: File, chunkCount: number): Promise<string> {
    const fileId = fileIdFor(file);
    await this.db.put("files", {
      fileId,
      fileName: file.name,
      fileSize: file.size,
      lastOpened: Date.now(),
      chunkCount,
      blob: file,
    });
    return fileId;
  }

  listFiles(): Promise<StoredLog[]> {
    return this.db.getAll("files");
  }
  deleteFile(fileId: string): Promise<void> {
    return this.db.delete("files", fileId);
  }

  async getFile(fileId: string): Promise<StoredLog> {
    const stored = await this.db.get("files", fileId);
    if (!stored) throw new Error("The saved log was not found.");
    return stored;
  }
}
export default EvtxStorage;
