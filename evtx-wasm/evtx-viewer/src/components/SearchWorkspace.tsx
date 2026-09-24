import { useEffect, useMemo, useRef, useState } from "react";
import { styled } from "styled-components";
import {
  Delete16Regular,
  Dismiss16Regular,
  QuestionCircle16Regular,
  Save16Regular,
  Search16Regular,
} from "@fluentui/react-icons";
import { useFilters } from "../hooks/useFilters";
import { useColumns } from "../hooks/useColumns";
import { parseSearchQuery, SEARCH_FIELDS } from "../lib/searchQuery";
import { errorMessage } from "../lib/types";
import {
  readSavedViews,
  restoreColumns,
  SAVED_VIEWS_KEY,
  writeSavedViews,
  type SavedView,
} from "../lib/savedViews";
import { buildFacetConfigs } from "./FilterSidebar/facetUtils";
import { useActiveFilterChips } from "./FilterSidebar/useActiveFilterChips";
import { Button, Input, Popover, Select, SearchContainer, SearchInput, Tooltip } from "./Windows";

const Workspace = styled.section`
  flex-shrink: 0;
  background: ${({ theme }) => theme.colors.surface.pane};
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
`;
const SearchRow = styled.form`
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 8px;
  min-width: 0;
`;
const QueryField = styled.div`
  position: relative;
  flex: 1 1 auto;
  min-width: 120px;
`;
const QueryBox = styled(SearchContainer)`
  padding-right: 2px;
  color: ${({ theme }) => theme.colors.text.secondary};
  input {
    color: ${({ theme }) => theme.colors.text.primary};
  }
`;
// Overlays the grid below the box, so an error never moves the table.
const QueryError = styled.div`
  position: absolute;
  z-index: 20;
  top: calc(100% + 2px);
  left: 0;
  max-width: 100%;
  padding: 3px 8px;
  border: 1px solid ${({ theme }) => theme.colors.severity.error};
  border-radius: ${({ theme }) => theme.radius.control};
  background: ${({ theme }) => theme.colors.surface.pane};
  color: ${({ theme }) => theme.colors.severity.error};
  box-shadow: ${({ theme }) => theme.shadow.flyout};
  pointer-events: none;
`;
const IconButton = styled(Button)`
  width: 24px;
  padding: 0;
  color: ${({ theme }) => theme.colors.text.secondary};
`;
const NameInput = styled(Input)`
  flex: 1;
`;
const SavedSelect = styled(Select)`
  flex: 0 1 168px;
  min-width: 96px;
`;
const Message = styled.p`
  padding: 0 8px 4px;
  color: ${({ theme }) => theme.colors.severity.error};
`;
const Chips = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  padding: 0 8px 4px;
`;
const Chip = styled.span`
  display: inline-flex;
  align-items: center;
  gap: 2px;
  max-width: 360px;
  height: ${({ theme }) => theme.size.row};
  padding: 0 2px 0 8px;
  border: 1px solid ${({ theme }) => theme.colors.stroke.control};
  border-radius: ${({ theme }) => theme.radius.control};
  background: ${({ theme }) => theme.colors.fill.hover};
  > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
`;
const ChipDismiss = styled.button`
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: 0;
  border-radius: 2px;
  background: transparent;
  color: ${({ theme }) => theme.colors.text.secondary};
  cursor: default;
  &:hover:not(:disabled) {
    background: ${({ theme }) => theme.colors.fill.hover};
    color: ${({ theme }) => theme.colors.text.primary};
  }
`;
const Help = styled.section`
  width: 360px;
  padding: 8px;
  h2 {
    font-size: inherit;
    font-weight: 600;
    margin: 0 0 4px;
  }
  h2 + dl {
    margin-bottom: 12px;
  }
  dl {
    display: grid;
    grid-template-columns: 136px 1fr;
    gap: 4px 12px;
  }
  dt code {
    font-family: ${({ theme }) => theme.fonts.mono};
  }
  dd {
    color: ${({ theme }) => theme.colors.text.secondary};
  }
`;

const SYNTAX: [string, string][] = [
  ["event_id:4624", `Field match. Fields: ${SEARCH_FIELDS.join(", ")}`],
  ["@TargetUserName:bob", "Event data field"],
  ["-channel:Security", "Exclude matches"],
  ['"logon type"', "Quote values with spaces"],
  ["failed", "Other words match anywhere in the event"],
];
const SHORTCUTS: [string, string][] = [
  ["Ctrl/⌘+O", "Open log"],
  ["Ctrl/⌘+S", "Save original log"],
  ["F5", "Refresh"],
  ["Enter", "Apply search now"],
];

export function SearchWorkspace({ disabled = false }: { disabled?: boolean }) {
  const { filters, setFilters, updateFilters, clearFilters } = useFilters();
  const { columns, setColumns } = useColumns();
  const [draft, setDraft] = useState(filters.searchQuery ?? "");
  const [draftFilters, setDraftFilters] = useState(filters);
  const [helpAnchor, setHelpAnchor] = useState<HTMLElement | null>(null);
  if (draftFilters !== filters) {
    setDraftFilters(filters);
    if (draftFilters.searchQuery !== filters.searchQuery || Object.keys(filters).length === 0)
      setDraft(filters.searchQuery ?? "");
  }
  const [saveName, setSaveName] = useState<string | null>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  const saving = saveName !== null;
  useEffect(() => {
    if (saving) nameInput.current?.focus();
  }, [saving]);
  const [selectedView, setSelectedView] = useState("");
  const [storageError, setStorageError] = useState("");
  const [views, setViews] = useState<SavedView[]>(() => {
    try {
      return readSavedViews(localStorage.getItem(SAVED_VIEWS_KEY));
    } catch {
      return [];
    }
  });
  const queryError = useMemo(() => {
    try {
      parseSearchQuery(draft);
      return "";
    } catch (error) {
      return errorMessage(error);
    }
  }, [draft]);

  const searchQuery = filters.searchQuery ?? "";
  useEffect(() => {
    if (disabled || queryError || draft === searchQuery) return;
    const timer = setTimeout(
      () => updateFilters((current) => ({ ...current, searchQuery: draft })),
      250,
    );
    return () => clearTimeout(timer);
  }, [draft, disabled, queryError, searchQuery, updateFilters]);

  const selected = views.find((view) => view.name === selectedView);
  const displayedSelection =
    selected &&
    draft === (selected.filters.searchQuery ?? "") &&
    JSON.stringify(filters) === JSON.stringify(selected.filters) &&
    JSON.stringify(columns.map(({ id, width }) => ({ id, width }))) ===
      JSON.stringify(
        restoreColumns(selected.columns).map(({ id, width }) => ({
          id,
          width,
        })),
      )
      ? selectedView
      : "";

  const facets = useMemo(() => buildFacetConfigs(columns), [columns]);
  const chips = useActiveFilterChips(facets);

  function persist(next: SavedView[]) {
    try {
      const serialized = writeSavedViews(next);
      localStorage.setItem(SAVED_VIEWS_KEY, serialized);
      setViews(next);
      setStorageError("");
      return true;
    } catch (error) {
      setStorageError(
        `Could not save searches on this device. ${errorMessage(error, "Check browser storage settings.")}`,
      );
      return false;
    }
  }

  return (
    <Workspace aria-label="Search and saved searches">
      <SearchRow
        onSubmit={(event) => {
          event.preventDefault();
          if (!queryError && !disabled)
            updateFilters((current) => ({ ...current, searchQuery: draft }));
        }}
      >
        <QueryField>
          <QueryBox>
            <Search16Regular aria-hidden="true" />
            <SearchInput
              id="event-search"
              aria-label="Search events"
              aria-describedby={queryError ? "event-search-error" : undefined}
              aria-invalid={!!queryError}
              disabled={disabled}
              autoComplete="off"
              spellCheck={false}
              placeholder="Search events, e.g. event_id:4624 -user:SYSTEM"
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
            />
            {draft && (
              <Tooltip label="Clear search">
                <IconButton
                  type="button"
                  variant="subtle"
                  aria-label="Clear search"
                  disabled={disabled}
                  onClick={() => {
                    setDraft("");
                    updateFilters((current) => ({ ...current, searchQuery: "" }));
                  }}
                  icon={<Dismiss16Regular />}
                />
              </Tooltip>
            )}
          </QueryBox>
          {queryError && (
            <QueryError id="event-search-error" role="alert">
              {queryError}
            </QueryError>
          )}
        </QueryField>
        <Tooltip label="Search syntax and shortcuts">
          <IconButton
            id="search-help"
            type="button"
            variant="subtle"
            aria-label="Search syntax and shortcuts"
            aria-expanded={!!helpAnchor}
            icon={<QuestionCircle16Regular />}
            onClick={(event) => {
              const button = event.currentTarget;
              setHelpAnchor((open) => (open ? null : button));
            }}
          />
        </Tooltip>
        <SavedSelect
          aria-label="Saved searches"
          disabled={disabled || !views.length}
          value={displayedSelection}
          onChange={(event) => {
            setSelectedView(event.target.value);
            const view = views.find((item) => item.name === event.target.value);
            if (view) {
              setFilters(view.filters);
              setColumns(restoreColumns(view.columns));
              setDraft(view.filters.searchQuery ?? "");
            }
          }}
        >
          <option value="">Saved searches</option>
          {views.map((view) => (
            <option key={view.name} value={view.name}>
              {view.name}
            </option>
          ))}
        </SavedSelect>
        {displayedSelection && (
          <Tooltip label="Delete saved search">
            <IconButton
              type="button"
              variant="subtle"
              aria-label="Delete saved search"
              icon={<Delete16Regular />}
              onClick={() => {
                if (persist(views.filter((view) => view.name !== selectedView)))
                  setSelectedView("");
              }}
            />
          </Tooltip>
        )}
        <Button
          type="button"
          disabled={disabled || !!queryError}
          icon={<Save16Regular />}
          onClick={() => setSaveName("")}
        >
          Save search
        </Button>
      </SearchRow>
      {helpAnchor && (
        <Popover anchor={helpAnchor} onClose={() => setHelpAnchor(null)}>
          <Help aria-label="Search syntax and shortcuts">
            <h2>Search syntax</h2>
            <dl>
              {SYNTAX.map(([example, meaning]) => (
                <div key={example} style={{ display: "contents" }}>
                  <dt>
                    <code>{example}</code>
                  </dt>
                  <dd>{meaning}</dd>
                </div>
              ))}
            </dl>
            <h2>Keyboard shortcuts</h2>
            <dl>
              {SHORTCUTS.map(([keys, action]) => (
                <div key={keys} style={{ display: "contents" }}>
                  <dt>{keys}</dt>
                  <dd>{action}</dd>
                </div>
              ))}
            </dl>
          </Help>
        </Popover>
      )}
      {saveName !== null && (
        <SearchRow
          onSubmit={(event) => {
            event.preventDefault();
            const name = saveName.trim();
            if (!name || queryError) return;
            if (views.length >= 50 && !views.some((view) => view.name === name)) {
              setStorageError("You can save up to 50 searches. Delete one before saving another.");
              return;
            }
            const view: SavedView = {
              name,
              filters: { ...filters, searchQuery: draft },
              columns: columns.map(({ id, width }) => ({ id, width })),
            };
            if (persist([...views.filter((item) => item.name !== name), view])) {
              setFilters(view.filters);
              setSaveName(null);
              setSelectedView(name);
            }
          }}
        >
          <label htmlFor="saved-search-name">Search name</label>
          <NameInput
            ref={nameInput}
            id="saved-search-name"
            required
            maxLength={80}
            value={saveName}
            onChange={(event) => setSaveName(event.target.value)}
          />
          <Button type="submit" disabled={!saveName.trim() || !!queryError}>
            {views.some((view) => view.name === saveName.trim()) ? "Replace saved search" : "Save"}
          </Button>
          <Button type="button" variant="subtle" onClick={() => setSaveName(null)}>
            Cancel
          </Button>
        </SearchRow>
      )}
      {storageError && <Message role="alert">{storageError}</Message>}
      {chips.length > 0 && (
        <Chips aria-label="Active filters">
          {chips.map((chip) => (
            <Chip key={chip.key} title={chip.label}>
              <span>{chip.label}</span>
              <ChipDismiss
                type="button"
                aria-label={`Remove ${chip.label}`}
                onClick={chip.remove}
                disabled={disabled}
              >
                <Dismiss16Regular aria-hidden="true" />
              </ChipDismiss>
            </Chip>
          ))}
          <Button
            type="button"
            variant="subtle"
            disabled={disabled}
            onClick={() => {
              setDraft("");
              clearFilters();
            }}
          >
            Clear filters
          </Button>
        </Chips>
      )}
    </Workspace>
  );
}
