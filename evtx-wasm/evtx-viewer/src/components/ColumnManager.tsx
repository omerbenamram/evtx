import React, { useMemo, useState, useEffect } from "react";
import styled from "styled-components";
import type { TableColumn } from "../lib/types";
import {
  Panel,
  PanelBody,
  SidebarHeader,
  Button,
  ScrollableList,
  SelectableRow,
} from "./Windows";

// Locally override alignment so checkbox + label sit together on the left
const Row = styled(SelectableRow)`
  justify-content: flex-start;
`;

// Reuse FilterSidebar typographic scale & colors

const SidebarBody = styled(PanelBody)`
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: ${({ theme }) => theme.spacing.sm};
  background: ${({ theme }) => theme.colors.background.secondary};
`;

const SearchInput = styled.input`
  width: 100%;
  margin-bottom: ${({ theme }) => theme.spacing.sm};
  padding: 4px 6px;
  background: ${({ theme }) => theme.colors.background.secondary};
  border: 1px solid ${({ theme }) => theme.colors.border.light};
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  font-size: ${({ theme }) => theme.fontSize.caption};
  color: ${({ theme }) => theme.colors.text.primary};

  &::placeholder {
    color: ${({ theme }) => theme.colors.text.tertiary};
  }
`;

type Props = {
  allColumns: TableColumn[];
  active: TableColumn[];
  onChange: (next: TableColumn[]) => void;
  onClose: () => void;
  width?: number;
};

export const ColumnManager: React.FC<Props> = ({
  allColumns,
  active,
  onChange,
  onClose,
}) => {
  const [term, setTerm] = useState("");

  /**
   * Maintain a local superset of all columns ever seen during the component's
   * lifetime.  When the parent removes a column from the `active` array we
   * still want it to appear (unchecked) in the list so the user can add it
   * back later.  We merge any *new* columns received via props on every render
   * but **never** delete from this local list.
   */
  const [allCols, setAllCols] = useState<TableColumn[]>(allColumns);

  // Merge-in any new columns from props (e.g. dynamically added EventData cols)
  useEffect(() => {
    setAllCols((prev) => {
      const map = new Map(prev.map((c) => [c.id, c]));
      allColumns.forEach((c) => map.set(c.id, c));
      return Array.from(map.values());
    });
  }, [allColumns]);

  const activeIds = useMemo(() => new Set(active.map((c) => c.id)), [active]);

  const filtered = useMemo(() => {
    const t = term.toLowerCase();
    return allCols.filter((c) => c.header.toLowerCase().includes(t));
  }, [allCols, term]);

  const toggle = (col: TableColumn) => {
    if (activeIds.has(col.id)) {
      onChange(active.filter((c) => c.id !== col.id));
    } else {
      onChange([...active, col]);
    }
  };

  return (
    <Panel
      elevation="flat"
      padding="none"
      style={{ height: "100%", borderTop: "none" }}
    >
      <SidebarHeader>
        <span>Columns</span>
        <Button size="small" variant="subtle" onClick={onClose}>
          Close
        </Button>
      </SidebarHeader>
      <SidebarBody>
        <SearchInput
          placeholder="Search columns…"
          value={term}
          onChange={(e) => setTerm(e.target.value)}
        />
        <ScrollableList>
          {filtered.map((col) => (
            <Row key={col.id} $selected={activeIds.has(col.id)}>
              <input
                type="checkbox"
                checked={activeIds.has(col.id)}
                onChange={() => toggle(col)}
              />
              <span>{col.header}</span>
            </Row>
          ))}
        </ScrollableList>
      </SidebarBody>
    </Panel>
  );
};
