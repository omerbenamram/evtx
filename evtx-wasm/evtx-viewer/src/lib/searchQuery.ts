import { columnSql, EVENT_DATA_PREFIX, SYSTEM_COLUMN_IDS } from "./columnSql";
import { levelName, type FilterOptions } from "./types";

export const SEARCH_FIELDS = [
  "level",
  "event_id",
  "provider",
  "channel",
  "computer",
  "user",
  "task",
  "opcode",
  "keywords",
];

/** Query field (lowercase) -> column id. */
const FIELDS = new Map([
  ...SYSTEM_COLUMN_IDS.map((id) => [id.toLowerCase(), id] as const),
  ["event_id", "eventId"],
  ["id", "eventId"],
  ["source", "provider"],
  ["host", "computer"],
  ["log", "channel"],
]);

interface Token {
  value: string;
  negated: boolean;
  literal: boolean;
}

function tokenize(input: string): Token[] {
  const tokens: Token[] = [];
  let current: Token | undefined;
  let quoted = false;
  let escaped = false;
  for (const char of input.trim()) {
    if (escaped) {
      current!.value += char;
      escaped = false;
    } else if (quoted && char === "\\") {
      escaped = true;
    } else if (char === '"') {
      current ??= { value: "", negated: false, literal: false };
      if (!quoted && !current.value) current.literal = true;
      quoted = !quoted;
    } else if (!quoted && /\s/.test(char)) {
      if (current) tokens.push(current);
      current = undefined;
    } else if (!current) {
      current = { value: char === "-" ? "" : char, negated: char === "-", literal: false };
    } else {
      current.value += char;
    }
  }
  if (quoted || escaped) throw new Error("Close the quoted value before searching.");
  if (current) tokens.push(current);
  return tokens;
}

/** `field:value`, `@EventDataName:value`, `-` to exclude, other words match the raw event. */
export function parseSearchQuery(input: string): FilterOptions {
  const filters: FilterOptions = {};
  for (const { value, negated, literal } of tokenize(input)) {
    const [, field = "", raw = ""] = (!literal && /^(@?[\w.-]+):(.*)$/s.exec(value)) || [];
    // Unknown prefixes stay text so C:\Windows or http://host remain searchable.
    const id = field.startsWith("@")
      ? EVENT_DATA_PREFIX + field.slice(1)
      : FIELDS.get(field.toLowerCase());
    if (!id) {
      if (value) (filters[negated ? "notContains" : "contains"] ??= []).push(value);
      else if (negated) (filters.contains ??= []).push("-");
      continue;
    }
    if (!raw) throw new Error(`Enter a value after ${field}:`);
    const values = columnValues(id, raw);
    if (!values.length) throw new Error(`“${raw}” is not a valid ${field} value.`);
    ((filters[negated ? "exclude" : "include"] ??= {})[id] ??= []).push(...values);
  }
  return filters;
}

function columnValues(id: string, raw: string): string[] {
  if (id === "level" && !/^\d+$/.test(raw)) {
    const names = Array.from({ length: 6 }, (_, level) => String(level));
    return names.filter((level) => levelName(level).toLowerCase() === raw.toLowerCase());
  }
  const value = /^\d+$/.test(raw) ? String(Number(raw)) : raw;
  return columnSql(id)?.values?.test(value) === false ? [] : [value];
}
