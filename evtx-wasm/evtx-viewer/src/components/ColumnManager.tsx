import React, { useMemo, useState } from "react";
import { styled } from "styled-components";
import type { TableColumn } from "../lib/types";
import { useColumns } from "../hooks/useColumns";
import { SidebarHeader, Button, SelectableRow, Input } from "./Windows";

const Row = styled(SelectableRow)`
  justify-content: flex-start;
`;
const Container = styled.div`
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  min-height: 0;
  background: ${({ theme }) => theme.colors.background.secondary};
`;
const Body = styled.div`
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
  padding: ${({ theme }) => theme.spacing.sm};
`;
const List = styled.div`
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
`;

interface Props {
  onClose: () => void;
}

export const ColumnManager: React.FC<Props> = ({ onClose }) => {
  const { columns: active, allColumns, addColumn, removeColumn } = useColumns();
  const [term, setTerm] = useState("");

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
    <Container>
      <SidebarHeader>
        <span>Columns</span>
        <Button size="small" variant="subtle" onClick={onClose}>
          Close
        </Button>
      </SidebarHeader>
      <Body>
        <Input
          style={{ width: "100%", marginBottom: 8 }}
          aria-label="Search columns"
          placeholder="Search columns…"
          value={term}
          onChange={(e) => setTerm(e.target.value)}
        />
        <List>
          {filtered.map((col) => (
            <Row key={col.id} $selected={activeIds.has(col.id)}>
              <input
                type="checkbox"
                aria-label={col.header}
                disabled={active.length === 1 && activeIds.has(col.id)}
                checked={activeIds.has(col.id)}
                onChange={() => toggle(col)}
              />
              <span>{col.header}</span>
            </Row>
          ))}
        </List>
      </Body>
    </Container>
  );
};
