import { styled } from "styled-components";
import { Add16Regular, Subtract16Regular, Table16Regular } from "@fluentui/react-icons";
import { formatEventValue, getEventDataFields, levelName, type EvtxRecord } from "../lib/types";
import { buildEventDataColumn, formatDateTime } from "../lib/columns";
import { EVENT_DATA_PREFIX } from "../lib/columnSql";
import { toggleFacet } from "./FilterSidebar/facetUtils";
import { useFilters } from "../hooks/useFilters";
import { useColumns } from "../hooks/useColumns";
import { Button } from "./Windows";

const Pane = styled.section<{ $height: number }>`
  height: ${({ $height }) => $height}px;
  flex-shrink: 0;
  overflow: auto;
  padding: ${({ theme }) => theme.spacing.md};
  background: ${({ theme }) => theme.colors.background.secondary};
  color: ${({ theme }) => theme.colors.text.primary};
  h3 {
    font-size: ${({ theme }) => theme.fontSize.body};
    font-weight: 600;
    margin: 0 0 8px;
  }
  dl {
    margin-bottom: 16px;
  }
  pre {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
`;
const Field = styled.div`
  display: grid;
  grid-template-columns: minmax(130px, 180px) minmax(100px, 1fr) auto;
  align-items: start;
  gap: 6px 12px;
  padding: 3px 0;
  dt {
    color: ${({ theme }) => theme.colors.text.secondary};
    overflow-wrap: anywhere;
  }
  dd {
    min-width: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
`;
const Actions = styled.dd`
  display: flex;
  align-items: center;
  gap: 2px;
`;

interface Props {
  record: EvtxRecord;
  height: number;
}

export function EventDetailsPane({ record, height }: Props) {
  const { updateFilters } = useFilters();
  const { addColumn } = useColumns();
  const system = record.Event.System;
  const fields = record.Event.EventData ? getEventDataFields(record.Event.EventData) : [];
  const general = [
    ["Log name", system.Channel ?? "-"],
    ["Source", system.Provider_attributes?.Name ?? "-"],
    ["Event ID", formatEventValue(system.EventID)],
    ["Level", levelName(system.Level)],
    ["User", system.Security_attributes?.UserID ?? "-"],
    ["Logged (local time)", formatDateTime(system.TimeCreated_attributes?.SystemTime)],
    ["Computer", system.Computer ?? "-"],
  ];

  function filterField(name: string, value: string, map: "include" | "exclude") {
    updateFilters((current) =>
      toggleFacet(current, `${EVENT_DATA_PREFIX}${name}`, value, map, true),
    );
  }

  return (
    <Pane $height={height} aria-label="Selected event details">
      <h3>General</h3>
      <dl>
        {general.map(([name, value]) => (
          <Field key={name}>
            <dt>{name}</dt>
            <dd>{value}</dd>
          </Field>
        ))}
      </dl>
      <h3>Event data</h3>
      {fields.length ? (
        <dl>
          {fields.map((field, position) => (
            <Field key={`${position}:${field.name}`}>
              <dt>{field.name}</dt>
              <dd>{field.value}</dd>
              <Actions>
                <Button
                  size="small"
                  variant="subtle"
                  icon={<Add16Regular />}
                  title={`Include ${field.name}`}
                  aria-label={`Include ${field.name}`}
                  onClick={() => filterField(field.name, field.value, "include")}
                />
                <Button
                  size="small"
                  variant="subtle"
                  icon={<Subtract16Regular />}
                  title={`Exclude ${field.name}`}
                  aria-label={`Exclude ${field.name}`}
                  onClick={() => filterField(field.name, field.value, "exclude")}
                />
                <Button
                  size="small"
                  variant="subtle"
                  icon={<Table16Regular />}
                  title={`Add ${field.name} as column`}
                  aria-label={`Add ${field.name} as column`}
                  onClick={() => addColumn(buildEventDataColumn(field.name))}
                />
              </Actions>
            </Field>
          ))}
        </dl>
      ) : (
        <pre>
          {record.Event.UserData ? formatEventValue(record.Event.UserData) : "No event data"}
        </pre>
      )}
    </Pane>
  );
}
