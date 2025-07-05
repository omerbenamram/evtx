// Core types for EVTX parsing
export interface EvtxFileInfo {
  fileName: string;
  fileSize: number;
  totalChunks: number;
  /** May exceed JavaScript's safe integer range so we treat it as a string */
  nextRecordId: string;
  isDirty: boolean;
  isFull: boolean;
  chunks: ChunkInfo[];
}

export interface ChunkInfo {
  chunkNumber: number;
  recordCount: string;
  /** Potentially very large – keep as string */
  firstRecordId: string;
  lastRecordId: string;
}

export interface EvtxRecord {
  Event: {
    System: EvtxSystemData;
    EventData?: EvtxEventData | null;
    UserData?: unknown;
    RenderingInfo?: unknown;
  };
}

export interface EvtxSystemData {
  Provider?: {
    Name?: string;
    Guid?: string;
  };
  Provider_attributes?: {
    Name?: string;
    Guid?: string;
  };
  EventID?: number | string;
  Version?: number;
  Level?: number;
  Task?: number;
  Opcode?: number;
  Keywords?: string;
  TimeCreated?: {
    SystemTime?: string;
  };
  TimeCreated_attributes?: {
    SystemTime?: string;
  };
  EventRecordID?: number;
  Correlation?: unknown;
  Execution?: {
    ProcessID?: number;
    ThreadID?: number;
  };
  Execution_attributes?: {
    ProcessID?: number;
    ThreadID?: number;
  };
  Channel?: string;
  Computer?: string;
  Security?: {
    UserID?: string;
  };
  Security_attributes?: {
    UserID?: string;
  };
}

export interface EvtxEventData {
  Data?: DataElement | DataElement[];
  "#text"?: string;
  [key: string]: unknown;
}

export interface DataElement {
  "#text"?: string;
  "#attributes"?: {
    Name?: string;
  };
}

export interface ParseResult {
  records: EvtxRecord[];
  totalRecords: number;
  errors: string[];
}

export interface TableColumn {
  id: string;
  header: string;
  accessor: (record: EvtxRecord) => unknown;
  width?: number;
  sortable?: boolean;
}

export type ExportFormat = "json" | "xml";

export interface FilterOptions {
  searchTerm?: string;
  level?: number[];
  eventId?: number[];
  timeRange?: {
    start: Date;
    end: Date;
  };
  provider?: string[];
  channel?: string[];
}

// Pre-computed facet buckets across the entire log file
export interface BucketCounts {
  level: Record<string, number>;
  provider: Record<string, number>;
  channel: Record<string, number>;
  event_id: Record<string, number>;
}
