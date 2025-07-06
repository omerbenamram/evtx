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
  /** DuckDB select expression.  Must alias to the same `id` */
  sqlExpr: string;
  /** Optional value formatter – receives the raw value returned from SQL */
  accessor?: (row: Record<string, unknown>) => unknown;
  width?: number;
  sortable?: boolean;
}

/** Alias kept for clarity – identical to TableColumn */
export type ColumnSpec = TableColumn;

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
  /**
   * Filters applied to specific EventData fields.  The map key is the field
   * name (e.g. "SubjectUserSid") and the value is a list of accepted values
   * for that field.  All active EventData field filters are AND-ed together
   * in the query, matching any record where the field’s value equals one of
   * the provided strings.
   */
  eventData?: Record<string, string[]>;

  /**
   * Exclusion filters for EventData.  Records whose field value matches any
   * of the listed values will be filtered out.
   */
  eventDataExclude?: Record<string, string[]>;

  /** Generic equality filters keyed by column id (id refers to TableColumn.id). */
  columnEquals?: Record<string, string[]>;
}

// Pre-computed facet buckets across the entire log file
export interface BucketCounts {
  level: Record<string, number>;
  provider: Record<string, number>;
  channel: Record<string, number>;
  event_id: Record<string, number>;
}
