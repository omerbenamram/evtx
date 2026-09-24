import { useEffect, useMemo, useRef, useState } from "react";
import { styled } from "styled-components";
import {
  Dismiss16Regular,
  Save20Regular,
  Search20Regular,
  Info20Regular,
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
import { ActiveFiltersBar, FilterChip } from "./FilterSidebar/styles";
import { Button, Input, Select, SearchContainer, SearchInput } from "./Windows";

const Workspace = styled.section`
  flex-shrink: 0;
  background: ${({ theme }) => theme.colors.background.secondary};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
`;
const SearchRow = styled.form`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
`;
const QueryBox = styled(SearchContainer)`
  flex: 1 1 280px;
  min-width: 160px;
`;
const NameInput = styled(Input)`
  flex: 1;
`;
const SavedSelect = styled(Select)`
  width: 176px;
`;
const Message = styled.p<{ $error?: boolean }>`
  padding: 0 12px 8px;
  font-size: ${({ theme }) => theme.fontSize.caption};
  color: ${({ theme, $error }) => ($error ? theme.colors.status.error : theme.colors.text.secondary)};
`;

export function SearchWorkspace({ disabled = false }: { disabled?: boolean }) {
  const { filters, setFilters, updateFilters, clearFilters } = useFilters();
  const { columns, setColumns } = useColumns();
  const [draft, setDraft] = useState(filters.searchQuery ?? "");
  const [draftFilters, setDraftFilters] = useState(filters);
  const [showHelp, setShowHelp] = useState(false);
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
        <QueryBox>
          <Search20Regular aria-hidden="true" />
          <SearchInput
            id="event-search"
            aria-label="Search events"
            aria-describedby="event-search-help"
            aria-invalid={!!queryError}
            disabled={disabled}
            list="event-search-fields"
            autoComplete="off"
            spellCheck={false}
            placeholder="Search events or event_id:4624"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
          />
          {draft && (
            <Button
              type="button"
              size="small"
              variant="subtle"
              aria-label="Clear search"
              disabled={disabled}
              onClick={() => {
                setDraft("");
                updateFilters((current) => ({ ...current, searchQuery: "" }));
              }}
              icon={<Dismiss16Regular />}
            />
          )}
        </QueryBox>
        <datalist id="event-search-fields">
          {SEARCH_FIELDS.map((field) => {
            const prefix = draft.replace(/\S*$/, "");
            return (
              <option key={field} value={`${prefix}${field}:`}>
                {field}
              </option>
            );
          })}
        </datalist>
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
        <Button
          type="button"
          size="small"
          disabled={disabled || !!queryError}
          icon={<Save20Regular />}
          onClick={() => setSaveName("")}
        >
          Save search
        </Button>
        <Button
          size="small"
          variant="subtle"
          aria-label="Search syntax"
          aria-expanded={showHelp}
          title="Search syntax"
          icon={<Info20Regular />}
          onClick={() => setShowHelp((value) => !value)}
        />
        {displayedSelection && (
          <Button
            type="button"
            size="small"
            variant="subtle"
            onClick={() => {
              if (persist(views.filter((view) => view.name !== selectedView))) setSelectedView("");
            }}
          >
            Delete saved search
          </Button>
        )}
      </SearchRow>
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
          <Button type="submit" size="small" disabled={!saveName.trim() || !!queryError}>
            {views.some((view) => view.name === saveName.trim()) ? "Replace saved search" : "Save"}
          </Button>
          <Button type="button" size="small" variant="subtle" onClick={() => setSaveName(null)}>
            Cancel
          </Button>
        </SearchRow>
      )}
      <Message
        id="event-search-help"
        hidden={!showHelp && !queryError}
        $error={!!queryError}
        role={queryError ? "alert" : undefined}
      >
        {queryError ||
          `Fields: ${SEARCH_FIELDS.join(", ")}. Quote values with spaces. Plain text matches event data.`}
      </Message>
      {storageError && (
        <Message $error role="alert">
          {storageError}
        </Message>
      )}
      {chips.length > 0 && (
        <ActiveFiltersBar aria-label="Active filters">
          {chips.map((chip) => (
            <FilterChip key={chip.key}>
              {chip.label}
              <Button
                size="small"
                variant="subtle"
                aria-label={`Remove ${chip.label}`}
                onClick={chip.remove}
                disabled={disabled}
              >
                <Dismiss16Regular aria-hidden="true" />
              </Button>
            </FilterChip>
          ))}
          <Button
            type="button"
            size="small"
            variant="subtle"
            disabled={disabled}
            onClick={() => {
              setDraft("");
              clearFilters();
            }}
          >
            Clear filters
          </Button>
        </ActiveFiltersBar>
      )}
    </Workspace>
  );
}
