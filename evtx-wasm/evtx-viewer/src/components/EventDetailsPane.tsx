import { useState, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { styled } from "styled-components";
import { z } from "zod";
import {
  Copy16Regular,
  TableAdd16Regular,
  ZoomIn16Regular,
  ZoomOut16Regular,
} from "@fluentui/react-icons";
import { formatEventValue, getEventDataFields, levelName, type EvtxRecord } from "../lib/types";
import { buildEventDataColumn, getDefaultColumns } from "../lib/columns";
import { EVENT_DATA_PREFIX } from "../lib/columnSql";
import { formatEventTime, timeZoneLabel, useTimeZone } from "../lib/timeZone";
import { toggleFacet } from "./FilterSidebar/facetUtils";
import { useFilters } from "../hooks/useFilters";
import { useColumns } from "../hooks/useColumns";
import { Button, ContextMenu, Tooltip } from "./Windows";

const Pane = styled.section<{ $height: number }>`
  display: flex;
  flex-direction: column;
  height: ${({ $height }) => $height}px;
  flex-shrink: 0;
  min-height: 0;
  background: ${({ theme }) => theme.colors.surface.pane};
  color: ${({ theme }) => theme.colors.text.primary};
`;
export const TitleBand = styled.h2`
  flex: 0 0 ${({ theme }) => theme.size.header};
  display: flex;
  align-items: center;
  gap: 16px;
  height: ${({ theme }) => theme.size.header};
  margin: 0;
  padding: 0 8px;
  overflow: hidden;
  background: ${({ theme }) => theme.colors.band.background};
  color: ${({ theme }) => theme.colors.band.text};
  font-size: ${({ theme }) => theme.fontSize.body};
  font-weight: 600;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-variant-numeric: tabular-nums;
  @media (forced-colors: active) {
    border-bottom: 1px solid CanvasText;
  }
`;
const Tabs = styled.div`
  flex: 0 0 auto;
  display: flex;
  gap: 4px;
  padding: 0 4px;
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
`;
const Tab = styled.button`
  position: relative;
  height: ${({ theme }) => theme.size.control};
  padding: 0 8px;
  border: 0;
  background: transparent;
  color: ${({ theme }) => theme.colors.text.secondary};
  cursor: default;
  &:hover {
    color: ${({ theme }) => theme.colors.text.primary};
  }
  &[aria-selected="true"] {
    color: ${({ theme }) => theme.colors.text.primary};
    font-weight: 600;
    &::after {
      content: "";
      position: absolute;
      left: 8px;
      right: 8px;
      bottom: 0;
      height: 2px;
      border-radius: 1px;
      background: ${({ theme }) => theme.colors.accent.rest};
    }
  }
  &:focus-visible {
    outline: 1px solid ${({ theme }) => theme.colors.focus};
    outline-offset: -3px;
  }
`;
const Panel = styled.div`
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 4px 0;
  scrollbar-width: thin;
`;
const List = styled.dl`
  margin: 0;
`;
const Field = styled.div`
  display: grid;
  grid-template-columns: minmax(120px, 180px) minmax(0, 1fr);
  column-gap: 12px;
  min-height: ${({ theme }) => theme.size.row};
  padding: 3px 8px;
  box-sizing: border-box;
  line-height: 16px;
  &:hover {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  dt {
    color: ${({ theme }) => theme.colors.text.secondary};
    overflow-wrap: anywhere;
  }
  dd {
    margin: 0;
    min-width: 0;
  }
`;
const Value = styled.span<{ $mono: boolean }>`
  font-family: ${({ theme, $mono }) => ($mono ? theme.fonts.mono : "inherit")};
  overflow-wrap: anywhere;
  user-select: text;
  font-variant-numeric: tabular-nums;
`;
const Group = styled.div`
  padding: 8px 8px 2px;
  color: ${({ theme }) => theme.colors.text.secondary};
  font-weight: 600;
`;
const Actions = styled.span`
  display: inline-flex;
  gap: 0;
  margin-left: 6px;
  vertical-align: top;
  opacity: 0;
  ${Field}:hover &,
  ${Field}:focus-within & {
    opacity: 1;
  }
`;
const ActionButton = styled(Button).attrs({ variant: "subtle" })`
  height: 16px;
  min-width: 20px;
  padding: 0 2px;
  color: ${({ theme }) => theme.colors.text.secondary};
`;
const Raw = styled.pre`
  margin: 0;
  padding: 4px 8px;
  font-family: ${({ theme }) => theme.fonts.mono};
  font-size: ${({ theme }) => theme.fontSize.body};
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  user-select: text;
`;

interface Row {
  name: string;
  value: string;
  mono?: boolean;
  /** Column the value filters; absent = copy only. */
  column?: string;
  /** null = the value is not set: include matches "not set", exclude is unavailable. */
  filterValue?: string | null;
}

// SIDs, hex and GUIDs read as raw values.
const RAW_VALUE = /^(S-\d-|0x[\da-f]+$|\{[\da-f-]{36}\}$)/i;

const optional = (value: string | null | undefined) => ({
  value: value ?? "-",
  filterValue: value ?? null,
});
// An unnamed <Data> item with no text renders as {"#text": null}.
const unsetData = z.object({ "#text": z.null() });

interface Action {
  id: string;
  label: string;
  icon: ReactNode;
  disabled: boolean;
  onClick: () => void;
}

interface Props {
  record: EvtxRecord;
  height: number;
}

export function EventDetailsPane({ record, height }: Props) {
  const { updateFilters } = useFilters();
  const { columns, addColumn } = useColumns();
  const zone = useTimeZone();
  const [tab, setTab] = useState<"general" | "details">("general");
  const [menu, setMenu] = useState<{ items: Action[]; x: number; y: number } | null>(null);
  const system = record.Event.System;
  const data = record.Event.EventData;
  const provider = system.Provider_attributes?.Name;
  const eventId = formatEventValue(system.EventID);
  const general: Row[] = [
    { name: "Log Name", column: "channel", ...optional(system.Channel) },
    { name: "Source", column: "provider", ...optional(provider) },
    { name: "Event ID", column: "eventId", value: eventId, filterValue: eventId },
    {
      name: "Level",
      column: "level",
      value: levelName(system.Level),
      filterValue:
        system.Level === null || system.Level === undefined ? null : String(system.Level),
    },
    { name: "User", column: "user", mono: true, ...optional(system.Security_attributes?.UserID) },
    {
      name: `Logged (${timeZoneLabel(zone)})`,
      value: formatEventTime(system.TimeCreated_attributes?.SystemTime, zone),
    },
    { name: "Computer", column: "computer", ...optional(system.Computer) },
  ];
  const eventData: Row[] = (data ? getEventDataFields(data) : []).map(({ name, value }) => {
    const raw = data?.[name];
    const unset = raw === null || raw === undefined || unsetData.safeParse(raw).success;
    return {
      name,
      value,
      mono: true,
      column: `${EVENT_DATA_PREFIX}${name}`,
      filterValue: unset ? null : value,
    };
  });

  function actions(row: Row): Action[] {
    const column = row.column;
    const shown = columns.some((item) => item.id === column);
    const definition = column?.startsWith(EVENT_DATA_PREFIX)
      ? buildEventDataColumn(row.name)
      : getDefaultColumns().find((item) => item.id === column);
    const filter = (map: "include" | "exclude") => () =>
      column &&
      updateFilters((current) => toggleFacet(current, column, row.filterValue ?? "", map, true));
    return [
      {
        id: "include",
        label: "Filter in",
        icon: <ZoomIn16Regular />,
        disabled: !column,
        onClick: filter("include"),
      },
      {
        id: "exclude",
        label: "Filter out",
        icon: <ZoomOut16Regular />,
        // Excluding a value keeps rows where it is not set, so "not set" cannot be filtered out.
        disabled: !column || row.filterValue === null,
        onClick: filter("exclude"),
      },
      {
        id: "column",
        label: "Add as column",
        icon: <TableAdd16Regular />,
        disabled: !definition || shown,
        onClick: () => definition && addColumn(definition),
      },
      {
        id: "copy",
        label: "Copy value",
        icon: <Copy16Regular />,
        disabled: false,
        onClick: () => void navigator.clipboard.writeText(row.value).catch(() => undefined),
      },
    ];
  }

  const renderRow = (row: Row, key: string) => {
    const items = actions(row);
    return (
      <Field
        key={key}
        onContextMenu={(event: MouseEvent) => {
          event.preventDefault();
          setMenu({ items, x: event.clientX, y: event.clientY });
        }}
      >
        <dt>{row.name}</dt>
        <dd>
          <Value $mono={Boolean(row.mono) || RAW_VALUE.test(row.value)}>{row.value}</Value>
          <Actions>
            {items
              .filter((item) => !item.disabled)
              .map((item) => (
                <Tooltip key={item.id} label={`${item.label}: ${row.name}`}>
                  <ActionButton
                    icon={item.icon}
                    aria-label={`${item.label}: ${row.name}`}
                    onClick={item.onClick}
                  />
                </Tooltip>
              ))}
          </Actions>
        </dd>
      </Field>
    );
  };

  const switchTab = (event: KeyboardEvent) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const next = tab === "general" ? "details" : "general";
    setTab(next);
    document.getElementById(`event-tab-${next}`)?.focus();
  };

  return (
    <Pane $height={height} aria-label="Selected event details">
      <TitleBand title={`Event ${eventId}, ${provider ?? "-"}`}>
        Event {eventId}, {provider ?? "-"}
      </TitleBand>
      <Tabs role="tablist" aria-label="Event details view" onKeyDown={switchTab}>
        {(["general", "details"] as const).map((id) => (
          <Tab
            key={id}
            id={`event-tab-${id}`}
            role="tab"
            type="button"
            aria-selected={tab === id}
            aria-controls="event-tab-panel"
            tabIndex={tab === id ? 0 : -1}
            onClick={() => setTab(id)}
          >
            {id === "general" ? "General" : "Details"}
          </Tab>
        ))}
      </Tabs>
      <Panel id="event-tab-panel" role="tabpanel" aria-labelledby={`event-tab-${tab}`} tabIndex={0}>
        {tab === "general" ? (
          <>
            <List>{general.map((row) => renderRow(row, row.name))}</List>
            <Group>Event data</Group>
            {eventData.length ? (
              <List>{eventData.map((row, index) => renderRow(row, `${index}:${row.name}`))}</List>
            ) : (
              <Raw>
                {record.Event.UserData ? formatEventValue(record.Event.UserData) : "No event data"}
              </Raw>
            )}
          </>
        ) : (
          <Raw>{JSON.stringify(record, null, 2)}</Raw>
        )}
      </Panel>
      {menu && (
        <ContextMenu
          items={menu.items}
          position={{ x: menu.x, y: menu.y }}
          onClose={() => setMenu(null)}
          ariaLabel="Value actions"
        />
      )}
    </Pane>
  );
}
