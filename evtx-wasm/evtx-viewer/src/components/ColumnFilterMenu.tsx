import { useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { styled } from "styled-components";
import { Popover } from "./Windows";

export interface FilterValue {
  value: string;
  label: string;
  count: number;
}

interface Props {
  anchor: HTMLElement;
  label: string;
  /** null while the values load. */
  values: FilterValue[] | null;
  included: string[];
  filtered: boolean;
  onToggle: (value: string) => void;
  onClear: () => void;
  onClose: () => void;
}

const SEARCH_THRESHOLD = 12;

const Body = styled.fieldset`
  min-width: 0;
  margin: 0;
  padding: 0;
  border: 0;
  outline: none;
  display: flex;
  flex-direction: column;
  width: 280px;
  max-height: min(420px, calc(100dvh - 16px));
`;
const Search = styled.input`
  height: ${({ theme }) => theme.size.control};
  margin: 0 0 4px;
  padding: 0 8px;
  border: 1px solid ${({ theme }) => theme.colors.stroke.control};
  border-bottom-color: ${({ theme }) => theme.colors.stroke.strong};
  border-radius: ${({ theme }) => theme.radius.control};
  background: ${({ theme }) => theme.colors.surface.pane};
  color: inherit;
  font: inherit;
  &:focus-visible {
    outline: none;
    border-bottom: 2px solid ${({ theme }) => theme.colors.accent.rest};
  }
`;
const List = styled.div`
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  scrollbar-width: thin;
`;
const Row = styled.label`
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 28px;
  padding: 0 8px;
  border-radius: ${({ theme }) => theme.radius.control};
  cursor: default;
  &:hover,
  &:focus-within {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  input {
    flex: 0 0 auto;
    margin: 0;
    accent-color: ${({ theme }) => theme.colors.accent.rest};
  }
  > span:first-of-type {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
`;
const Count = styled.span`
  color: ${({ theme }) => theme.colors.text.tertiary};
  font-size: ${({ theme }) => theme.fontSize.secondary};
  font-variant-numeric: tabular-nums;
`;
const Note = styled.div`
  padding: 6px 8px;
  color: ${({ theme }) => theme.colors.text.tertiary};
`;
const Separator = styled.hr`
  height: 1px;
  border: 0;
  margin: 4px -4px;
  background: ${({ theme }) => theme.colors.stroke.divider};
`;
const Clear = styled.button`
  display: flex;
  align-items: center;
  min-height: 28px;
  padding: 0 8px 0 32px;
  border: 0;
  border-radius: ${({ theme }) => theme.radius.control};
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: default;
  &:hover:not(:disabled),
  &:focus-visible {
    outline: none;
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  &:disabled {
    color: ${({ theme }) => theme.colors.text.tertiary};
  }
`;

// Arrow keys move between the search field, the checkboxes and "Clear".
function navigate(event: KeyboardEvent<HTMLFieldSetElement>) {
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  const stops = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>("input, button:not(:disabled)"),
  );
  const at = stops.findIndex((stop) => stop === document.activeElement);
  const next = stops[at + (event.key === "ArrowDown" ? 1 : -1)];
  if (!next) return;
  event.preventDefault();
  next.focus();
}

/** A column's value checklist (checked = included), anchored to the header filter button. */
export function ColumnFilterMenu({
  anchor,
  label,
  values,
  included,
  filtered,
  onToggle,
  onClear,
  onClose,
}: Props) {
  const [query, setQuery] = useState("");
  const body = useRef<HTMLFieldSetElement>(null);
  const loaded = values !== null;
  // Runs after Popover has opened (parent effects run after the child's); again once values load.
  useLayoutEffect(() => {
    const element = body.current;
    if (!element) return;
    if (!loaded) element.focus();
    else if (document.activeElement === element || !element.contains(document.activeElement))
      element.querySelector<HTMLElement>("input, button:not(:disabled)")?.focus();
  }, [loaded]);
  const searchable = (values?.length ?? 0) > SEARCH_THRESHOLD;
  const needle = query.trim().toLowerCase();
  const shown = values?.filter((item) => !needle || item.label.toLowerCase().includes(needle));
  return (
    <Popover anchor={anchor} onClose={onClose}>
      <Body ref={body} tabIndex={-1} aria-label={label} onKeyDown={navigate}>
        {searchable && (
          <Search
            type="search"
            aria-label="Search values"
            placeholder="Search values"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        )}
        <List>
          {!shown ? (
            <Note>Loading values…</Note>
          ) : !shown.length ? (
            <Note>{values?.length ? "No matching values" : "No values"}</Note>
          ) : (
            shown.map((item) => (
              <Row key={item.value}>
                <input
                  type="checkbox"
                  checked={included.includes(item.value)}
                  onChange={() => onToggle(item.value)}
                />
                <span title={item.label}>{item.label}</span>
                <Count>{item.count.toLocaleString()}</Count>
              </Row>
            ))
          )}
        </List>
        <Separator />
        <Clear
          type="button"
          disabled={!filtered}
          onClick={() => {
            onClear();
            onClose();
          }}
        >
          Clear column filter
        </Clear>
      </Body>
    </Popover>
  );
}
