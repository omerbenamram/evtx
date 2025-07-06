import React from "react";
import styled from "styled-components";
import { useEvtxMetaState, useIngestState } from "../state/store";

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
  isWasmReady: boolean;
  isDuckDbReady: boolean;
}

export const StatusBar: React.FC<StatusBarProps> = ({
  isWasmReady,
  isDuckDbReady,
}) => {
  const { fileInfo, matchedCount, totalRecords } = useEvtxMetaState();
  const { progress: ingestProgress } = useIngestState();

  const eventCountDisplay = fileInfo
    ? `${fileInfo.fileName} - ${matchedCount}/${totalRecords || 0} events`
    : "No file loaded";

  const chunkCountDisplay = fileInfo ? `Chunks: ${fileInfo.totalChunks}` : null;

  let progressDisplay: string;

  if (!isWasmReady) {
    progressDisplay = "Loading WASM...";
  } else if (!isDuckDbReady) {
    progressDisplay = "Loading DB engine...";
  } else if (ingestProgress < 1 && fileInfo) {
    progressDisplay = `Loading DB ${
      ingestProgress * 100 < 0.01 && ingestProgress > 0
        ? 0.01
        : Math.round(ingestProgress * 10000) / 100
    }%`;
  } else {
    progressDisplay = "Ready";
  }

  return (
    <Bar>
      <Item>{eventCountDisplay}</Item>
      {chunkCountDisplay && <Item>{chunkCountDisplay}</Item>}
      <Item>{progressDisplay}</Item>
    </Bar>
  );
};
