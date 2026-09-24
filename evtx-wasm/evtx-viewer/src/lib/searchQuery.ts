import type { FilterOptions } from "./types";

/** Search field -> table column id. */
const SEARCH_COLUMNS = new Map([
  ["event_id", "eventId"],
  ["provider", "provider"],
  ["computer", "computer"],
  ["channel", "channel"],
]);
export const SEARCH_FIELDS = [...SEARCH_COLUMNS.keys()];

/** A small field syntax; values become filters, never SQL expressions. */
export function parseSearchQuery(input: string): FilterOptions {
  const tokens: { value: string; literal: boolean }[] = [];
  let token = "";
  let literal = false;
  let quoted = false;
  let escaped = false;
  for (const char of input.trim()) {
    if (escaped) {
      token += char;
      escaped = false;
    } else if (quoted && char === "\\") {
      escaped = true;
    } else if (char === '"') {
      if (!token && !quoted) literal = true;
      quoted = !quoted;
    } else if (/\s/.test(char) && !quoted) {
      if (token) tokens.push({ value: token, literal });
      token = "";
      literal = false;
    } else {
      token += char;
    }
  }
  if (quoted || escaped) throw new Error("Close the quoted value before searching.");
  if (token) tokens.push({ value: token, literal });

  const filters: FilterOptions = {};
  const text: string[] = [];
  for (const { value: part, literal: isLiteral } of tokens) {
    const [, rawField = "", value] = (!isLiteral && /^([a-z_]+):(.*)$/i.exec(part)) || [];
    const field = rawField.toLowerCase();
    // Unknown prefixes stay text so C:\Windows or http://host remain searchable.
    const column = SEARCH_COLUMNS.get(field);
    if (!column) {
      text.push(part);
      continue;
    }
    if (!value) throw new Error(`Enter a value after ${field}:`);
    if (column === "eventId" && (!/^\d+$/.test(value) || Number(value) > 65535)) {
      throw new Error("Event ID must be a whole number from 0 to 65535.");
    }
    ((filters.include ??= {})[column] ??= []).push(
      column === "eventId" ? String(Number(value)) : value,
    );
  }
  if (text.length) filters.searchTerm = text.join(" ");
  return filters;
}
