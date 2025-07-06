import React from "react";
import styled from "styled-components";
import { type EvtxFileInfo } from "../lib/types";

const Bar = styled.div`
  height: 24px;
  background: ${({ theme }) => theme.colors.background.secondary};
  border-top: 1px solid ${({ theme }) => theme.colors.border.light};
  display: flex;
  align-items: center;
  padding: 0 ${({ theme }) => theme.spacing.sm};
  font-size: ${({ theme }) => theme.fontSize.caption};
  color: ${({ theme }) => theme.colors.text.secondary};
  gap: ${({ theme }) => theme.spacing.lg};
`;

const Item = styled.span`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.xs};
`;

interface StatusBarProps {
  fileInfo: EvtxFileInfo | null;
  matchedCount: number;
  totalRecords: number;
  ingestProgress: number;
  isWasmReady: boolean;
}

export const StatusBar: React.FC<StatusBarProps> = ({
  fileInfo,
  matchedCount,
  totalRecords,
  ingestProgress,
  isWasmReady,
}) => {
  const eventCountDisplay = fileInfo
    ? `${fileInfo.fileName} - ${matchedCount}/${totalRecords || 0} events`
    : "No file loaded";

  const chunkCountDisplay = fileInfo ? `Chunks: ${fileInfo.totalChunks}` : null;

  const progressDisplay =
    ingestProgress < 1 && fileInfo
      ? `Loading DB ${
          ingestProgress * 100 < 0.01 && ingestProgress > 0
            ? 0.01
            : Math.round(ingestProgress * 10000) / 100
        }%`
      : isWasmReady
      ? "Ready"
      : "Loading WASM...";

  return (
    <Bar>
      <Item>{eventCountDisplay}</Item>
      {chunkCountDisplay && <Item>{chunkCountDisplay}</Item>}
      <Item>{progressDisplay}</Item>
    </Bar>
  );
};
