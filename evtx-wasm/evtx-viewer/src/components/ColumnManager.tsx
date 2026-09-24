import React, { useEffect, useMemo, useRef, useState } from "react";
import { styled } from "styled-components";
import type { TableColumn } from "../lib/types";
import { useColumns } from "../hooks/useColumns";
import { Input, Popover } from "./Windows";

const Panel = styled.section`
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 260px;
`;
const List = styled.div`
  max-height: min(420px, 60dvh);
  overflow: auto;
`;
const Row = styled.label`
  display: flex;
  align-items: center;
  gap: 8px;
  height: ${({ theme }) => theme.size.row};
  padding: 0 8px;
  border-radius: ${({ theme }) => theme.radius.control};
  cursor: default;
  &:hover {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  input {
    margin: 0;
  }
`;
const Empty = styled.p`
  padding: 4px 8px;
  color: ${({ theme }) => theme.colors.text.secondary};
`;

interface Props {
  anchor: HTMLElement;
  onClose: () => void;
}

export const ColumnManager: React.FC<Props> = ({ anchor, onClose }) => {
  const { columns: active, allColumns, addColumn, removeColumn } = useColumns();
  const [term, setTerm] = useState("");
  const search = useRef<HTMLInputElement>(null);
  // Runs after Popover has shown itself (child effects first).
  useEffect(() => search.current?.focus(), []);

  const activeIds = useMemo(() => new Set(active.map((c) => c.id)), [active]);

  const filtered = useMemo(() => {
    const t = term.toLowerCase();
    return allColumns.filter((c) => c.header.toLowerCase().includes(t));
  }, [allColumns, term]);

  const toggle = (column: TableColumn) => {
    if (activeIds.has(column.id)) removeColumn(column.id);
    else addColumn(column);
  };

  return (
    <Popover anchor={anchor} onClose={onClose}>
      <Panel aria-label="Table columns">
        <Input
          ref={search}
          aria-label="Search columns"
          placeholder="Search columns"
          value={term}
          onChange={(e) => setTerm(e.target.value)}
        />
        <List>
          {filtered.map((col) => (
            <Row key={col.id} title={col.header}>
              <input
                type="checkbox"
                disabled={active.length === 1 && activeIds.has(col.id)}
                checked={activeIds.has(col.id)}
                onChange={() => toggle(col)}
              />
              <span>{col.header}</span>
            </Row>
          ))}
          {!filtered.length && <Empty>No matching columns</Empty>}
        </List>
      </Panel>
    </Popover>
  );
};
