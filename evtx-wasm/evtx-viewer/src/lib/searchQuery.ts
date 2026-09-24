import { columnSql, EVENT_DATA_PREFIX, eventDataField, SYSTEM_COLUMN_IDS } from "./columnSql";
import { levelName, type ParsedQuery } from "./types";

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
  /** Index in `value` of the first unquoted colon. */
  colon: number;
  quoted: boolean;
  /** The token as typed, so edits keep the rest of the text. */
  source: string;
}

function tokenize(input: string): Token[] {
  const tokens: Token[] = [];
  let current: Token | undefined;
  let start = 0;
  let quoted = false;
  let escaped = false;
  const end = (at: number) => {
    if (current) tokens.push({ ...current, source: input.slice(start, at) });
    current = undefined;
  };
  for (let at = 0; at < input.length; at++) {
    const char = input[at];
    if (!current && !/\s/.test(char)) {
      start = at;
      current = { value: "", negated: char === "-", colon: -1, quoted: false, source: "" };
      if (char === "-") continue;
    }
    if (escaped) {
      current!.value += char;
      escaped = false;
    } else if (quoted && char === "\\") {
      escaped = true;
    } else if (char === '"') {
      current!.quoted = true;
      quoted = !quoted;
    } else if (!quoted && /\s/.test(char)) {
      end(at);
    } else if (current) {
      if (!quoted && char === ":" && current.colon < 0) current.colon = current.value.length;
      current.value += char;
    }
  }
  if (quoted || escaped) throw new Error("Close the quoted value before searching.");
  end(input.length);
  return tokens;
}

/** One `field:value`, `-field:value` or free-text word. `id` is undefined for free text. */
export interface QueryTerm {
  id?: string;
  /** Column values the term selects (level names expand), or the word itself. */
  values: string[];
  exclude: boolean;
  source: string;
}

/** `field:value`, `@EventDataName:value`, `field:""` for not set, `-` to exclude, other words. */
export function queryTerms(input: string): QueryTerm[] {
  return tokenize(input).map(({ value, negated, colon, quoted, source }) => {
    const field = colon < 0 ? "" : value.slice(0, colon);
    const raw = value.slice(colon + 1);
    // Unknown prefixes stay text so C:\Windows or http://host remain searchable.
    const id =
      field.length > 1 && field.startsWith("@")
        ? EVENT_DATA_PREFIX + field.slice(1)
        : FIELDS.get(field.toLowerCase());
    if (!id) return { values: [value || "-"], exclude: negated && !!value, source };
    // `field:` is unfinished; `field:""` is the explicit "not set".
    if (!raw && !quoted) throw new Error(`Enter a value after ${field}:`);
    const values = columnValues(id, raw);
    if (!values.length) throw new Error(`“${raw}” is not a valid ${field} value.`);
    return { id, values, exclude: negated, source };
  });
}

export function parseSearchQuery(input: string): ParsedQuery {
  const filters: ParsedQuery = {};
  for (const { id, values, exclude } of queryTerms(input)) {
    if (id) ((filters[exclude ? "exclude" : "include"] ??= {})[id] ??= []).push(...values);
    else (filters[exclude ? "notContains" : "contains"] ??= []).push(...values);
  }
  return filters;
}

function columnValues(id: string, raw: string): string[] {
  if (id === "level" && raw && !/^\d+$/.test(raw)) {
    const names = Array.from({ length: 6 }, (_, level) => String(level));
    return names.filter((level) => levelName(level).toLowerCase() === raw.toLowerCase());
  }
  const check = columnSql(id)?.values;
  // Only typed columns normalize numbers; text such as "0012" must round-trip exactly.
  const value = check && /^\d+$/.test(raw) ? String(Number(raw)) : raw;
  return value && check?.test(value) === false ? [] : [value];
}

const quote = (text: string, bare: RegExp) =>
  bare.test(text) ? text : `"${text.replace(/[\\"]/g, "\\$&")}"`;

/** The canonical text for one term: eventId -> event_id, eventData.X -> @X. */
export function formatTerm(id: string | undefined, value: string, exclude: boolean): string {
  const data = id && eventDataField(id);
  const field = data ? `@${quote(data, /^[\w.-]+$/)}` : id === "eventId" ? "event_id" : id;
  const text = quote(value, field ? /^[^\s":]+$/ : /^[^\s":-][^\s":]*$/);
  return `${exclude ? "-" : ""}${field ? `${field}:` : ""}${text}`;
}

const join = (terms: QueryTerm[]) => terms.map(({ source }) => source).join(" ");
const selects = (term: QueryTerm, id: string | undefined, value: string) =>
  term.id === id && term.values.includes(value);

// ponytail: a term matches if it selects the value, so removing level 4 also drops a typed
// `level:information` (0 and 4). Split such terms if that ever surprises anyone.
export function withoutTerm(
  query: string,
  id: string | undefined,
  value: string,
  exclude?: boolean,
): string {
  const terms = queryTerms(query);
  const kept = terms.filter(
    (term) => !selects(term, id, value) || (exclude !== undefined && term.exclude !== exclude),
  );
  return kept.length === terms.length ? query : join(kept);
}

/** Adds the term once, replacing its opposite (include vs exclude) for the same value. */
export function withTerm(
  query: string,
  id: string | undefined,
  value: string,
  exclude: boolean,
): string {
  const terms = queryTerms(query);
  if (terms.some((term) => selects(term, id, value) && term.exclude === exclude)) return query;
  const kept = terms.filter((term) => !selects(term, id, value));
  return join([...kept, { values: [value], exclude, source: formatTerm(id, value, exclude) }]);
}

export function withoutField(query: string, id: string): string {
  const terms = queryTerms(query);
  const kept = terms.filter((term) => term.id !== id);
  return kept.length === terms.length ? query : join(kept);
}

/**
 * Applies the committed change `before` -> `after` to a draft the user is editing: terms that
 * went away leave the draft and new ones join it. undefined when the draft does not parse.
 */
export function mergeQuery(draft: string, before: string, after: string): string | undefined {
  try {
    const old = queryTerms(before).map(({ source }) => source);
    const next = queryTerms(after).map(({ source }) => source);
    const kept = queryTerms(draft).filter(
      ({ source }) => !old.includes(source) || next.includes(source),
    );
    const added = next.filter(
      (source) => !old.includes(source) && !kept.some((term) => term.source === source),
    );
    return [...kept.map(({ source }) => source), ...added].join(" ");
  } catch {
    return undefined;
  }
}

/** Free-text words the query searches for (not the excluded ones), for highlighting. */
export function searchWords(query: string | undefined): string[] {
  try {
    return parseSearchQuery(query ?? "").contains ?? [];
  } catch {
    return [];
  }
}
