export const EVENT_DATA_PREFIX = "eventData.";

interface ColumnSql {
  sql: string;
  /** Column that `sql` formats for display; sorting, filtering and facet grouping use it. */
  typed?: string;
  /** Filter values other than "" that DuckDB can convert to this column's type. */
  values?: RegExp;
}

const INTEGER = /^-?\d+$/;
const SYSTEM_COLUMNS = new Map<string, ColumnSql>([
  ["level", { sql: "Level", values: INTEGER }],
  [
    "time",
    {
      sql: "strftime(TimeCreated, '%Y-%m-%dT%H:%M:%S.%fZ')",
      typed: "TimeCreated",
      values: /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{6}Z$/,
    },
  ],
  ["provider", { sql: "Provider" }],
  ["channel", { sql: "Channel" }],
  ["eventId", { sql: "EventID", values: /^\d{1,5}$/ }],
  ["task", { sql: "Task", values: INTEGER }],
  ["user", { sql: "UserID" }],
  ["computer", { sql: "Computer" }],
  ["opcode", { sql: "Opcode", values: INTEGER }],
  ["keywords", { sql: "Keywords" }],
]);

export const SYSTEM_COLUMN_IDS = [...SYSTEM_COLUMNS.keys()];

/** JSON Pointer treats dots and brackets in EventData names as literal text. */
export function eventDataExpression(field: string): string {
  // Unnamed <Data> items render as {"Data": {"#text": value}}; getEventDataFields shows the value.
  const name = field === "Data" ? "Data/#text" : field.replace(/~/g, "~0").replace(/\//g, "~1");
  return `json_extract_string(Raw, '${`/Event/EventData/${name}`.replace(/'/g, "''")}')`;
}

export function eventDataField(id: string): string | undefined {
  return id.startsWith(EVENT_DATA_PREFIX) ? id.slice(EVENT_DATA_PREFIX.length) : undefined;
}

export function columnSql(id: string): ColumnSql | undefined {
  const field = eventDataField(id);
  if (field !== undefined) return { sql: eventDataExpression(field) };
  return SYSTEM_COLUMNS.get(id);
}
