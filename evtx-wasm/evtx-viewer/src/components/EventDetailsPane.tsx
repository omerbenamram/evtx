import React from "react";
import styled, { css } from "styled-components";
import type { EvtxRecord, EvtxEventData, EvtxSystemData } from "../lib/types";

const DetailsPane = styled.div<{ $height: number }>`
  height: ${({ $height }) => `${$height}px`};
  border-top: 1px solid ${({ theme }) => theme.colors.border.medium};
  background: ${({ theme }) => theme.colors.background.secondary};
  padding: ${({ theme }) => theme.spacing.md};
  overflow-y: auto;
`;

const DetailSection = styled.div`
  margin-bottom: ${({ theme }) => theme.spacing.lg};
`;

const DetailTitle = styled.h3`
  font-size: ${({ theme }) => theme.fontSize.body};
  font-weight: 600;
  margin: 0 0 ${({ theme }) => theme.spacing.sm} 0;
  color: ${({ theme }) => theme.colors.text.primary};
`;

const DetailRow = styled.div`
  display: flex;
  margin-bottom: 4px;
`;

const DetailLabel = styled.span`
  font-weight: 600;
  min-width: 120px;
  color: ${({ theme }) => theme.colors.text.secondary};
`;

const DetailValue = styled.span`
  color: ${({ theme }) => theme.colors.text.primary};
`;

const DetailContent = styled.div`
  font-family: ${({ theme }) => theme.fonts.mono};
  font-size: ${({ theme }) => theme.fontSize.caption};
  color: ${({ theme }) => theme.colors.text.secondary};
  white-space: pre-wrap;
  word-break: break-word;
`;

const IconBtn = styled.button<{ $variant: "include" | "exclude" }>`
  width: 18px;
  height: 18px;
  margin-left: 4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid ${({ theme }) => theme.colors.border.light};
  border-radius: 3px;
  background: ${({ theme }) => theme.colors.background.secondary};
  font-size: 12px;
  line-height: 1;
  cursor: pointer;
  color: ${({ theme }) => theme.colors.text.secondary};
  &:hover {
    background: ${({ theme }) => theme.colors.background.hover};
  }
  &:active {
    background: ${({ theme }) => theme.colors.background.active};
  }
  ${(props) =>
    props.$variant === "include" &&
    css`
      /* could style differently if needed */
    `}
  ${(props) =>
    props.$variant === "exclude" &&
    css`
      /* maybe color accent */
    `}
`;

interface Props {
  record: EvtxRecord;
  height: number;
  /** When provided, clicking the “Include” button besides an EventData row
   * will invoke this callback so the parent can add a filter for that field.
   */
  onAddFilter?: (field: string, value: string) => void;
  /** Optional: add exclusion filter */
  onExcludeFilter?: (field: string, value: string) => void;
}

// Utility helpers copied from the original table
const LEVEL_NAMES: Record<number, string> = {
  0: "Information",
  1: "Critical",
  2: "Error",
  3: "Warning",
  4: "Information",
  5: "Verbose",
};

const formatDateTime = (systemTime?: string): string => {
  if (!systemTime) return "-";
  try {
    const date = new Date(systemTime);
    return date.toLocaleString("en-US", {
      month: "2-digit",
      day: "2-digit",
      year: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    });
  } catch {
    return systemTime;
  }
};

const getSystemData = (record: EvtxRecord): EvtxSystemData =>
  record.Event?.System || {};

const getEventId = (sys: EvtxSystemData): string => {
  const eid = sys.EventID;
  if (typeof eid === "object" && eid !== null) {
    return String((eid as Record<string, unknown>)["#text"] ?? "-");
  }
  return String(eid ?? "-");
};

const getProvider = (sys: EvtxSystemData): string =>
  sys.Provider?.Name || sys.Provider_attributes?.Name || "-";

const getTimeCreated = (sys: EvtxSystemData): string =>
  sys.TimeCreated?.SystemTime || sys.TimeCreated_attributes?.SystemTime || "";

const getUserId = (sys: EvtxSystemData): string =>
  sys.Security?.UserID || sys.Security_attributes?.UserID || "-";

const renderEventData = (
  eventData: unknown,
  onAdd?: (k: string, v: string) => void,
  onExclude?: (k: string, v: string) => void
): React.ReactNode => {
  if (!eventData) return "No event data";
  const eventObj = eventData as Record<string, unknown>;

  // Handle EventData/Data array style
  if (eventObj["Data"]) {
    const rawData = eventObj["Data"] as unknown;
    const dataArray = Array.isArray(rawData) ? rawData : [rawData];
    return dataArray.map((rawItem, idx) => {
      const item = rawItem as Record<string, unknown>;
      const name =
        (item["#attributes"] as Record<string, unknown> | undefined)?.Name ??
        `Data${idx}`;
      const value = item["#text"] ?? "-";
      const valueStr = String(value);
      return (
        <DetailRow key={idx}>
          <DetailLabel>{String(name)}:</DetailLabel>
          <DetailValue style={{ marginRight: 4 }}>{valueStr}</DetailValue>
          {onAdd && (
            <>
              <IconBtn
                $variant="include"
                title="Include"
                onClick={() => onAdd(String(name), valueStr)}
              >
                +
              </IconBtn>
              {onExclude && (
                <IconBtn
                  $variant="exclude"
                  title="Exclude"
                  onClick={() => onExclude(String(name), valueStr)}
                >
                  –
                </IconBtn>
              )}
            </>
          )}
        </DetailRow>
      );
    });
  }

  // Generic key/value pairs
  const kvRows = Object.entries(eventObj).map(([k, v]) => {
    const valStr = typeof v === "object" ? JSON.stringify(v) : String(v);
    return (
      <DetailRow key={k}>
        <DetailLabel>{k}:</DetailLabel>
        <DetailValue style={{ marginRight: 8 }}>{valStr}</DetailValue>
        {onAdd && (
          <IconBtn
            $variant="include"
            title="Include"
            onClick={() => onAdd(k, valStr)}
          >
            +
          </IconBtn>
        )}
        {onExclude && (
          <IconBtn
            $variant="exclude"
            title="Exclude"
            onClick={() => onExclude(k, valStr)}
          >
            –
          </IconBtn>
        )}
      </DetailRow>
    );
  });

  return kvRows.length > 0 ? kvRows : JSON.stringify(eventData, null, 2);
};

export const EventDetailsPane: React.FC<Props> = ({
  record,
  height,
  onAddFilter,
  onExcludeFilter,
}) => {
  const sys = getSystemData(record);
  return (
    <DetailsPane $height={height}>
      <DetailSection>
        <DetailTitle>General</DetailTitle>
        <DetailRow>
          <DetailLabel>Log Name:</DetailLabel>
          <DetailValue>{sys.Channel || "-"}</DetailValue>
        </DetailRow>
        <DetailRow>
          <DetailLabel>Source:</DetailLabel>
          <DetailValue>{getProvider(sys)}</DetailValue>
        </DetailRow>
        <DetailRow>
          <DetailLabel>Event ID:</DetailLabel>
          <DetailValue>{getEventId(sys)}</DetailValue>
        </DetailRow>
        <DetailRow>
          <DetailLabel>Level:</DetailLabel>
          <DetailValue>{LEVEL_NAMES[sys.Level || 4]}</DetailValue>
        </DetailRow>
        <DetailRow>
          <DetailLabel>User:</DetailLabel>
          <DetailValue>{getUserId(sys)}</DetailValue>
        </DetailRow>
        <DetailRow>
          <DetailLabel>Logged:</DetailLabel>
          <DetailValue>{formatDateTime(getTimeCreated(sys))}</DetailValue>
        </DetailRow>
        <DetailRow>
          <DetailLabel>Computer:</DetailLabel>
          <DetailValue>{sys.Computer || "-"}</DetailValue>
        </DetailRow>
      </DetailSection>

      {!!record.Event?.EventData && (
        <DetailSection>
          <DetailTitle>Event Data</DetailTitle>
          <DetailContent>
            {renderEventData(
              record.Event.EventData as EvtxEventData,
              onAddFilter,
              onExcludeFilter
            )}
          </DetailContent>
        </DetailSection>
      )}

      {!!record.Event?.UserData && (
        <DetailSection>
          <DetailTitle>User Data</DetailTitle>
          <pre style={{ whiteSpace: "pre-wrap" }}>
            {JSON.stringify(record.Event.UserData, null, 2)}
          </pre>
        </DetailSection>
      )}
    </DetailsPane>
  );
};
