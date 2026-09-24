import React from "react";
import { styled } from "styled-components";
import { useEvtxMetaState } from "../state/store";

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

export const StatusBar: React.FC = () => {
  const {
    fileInfo,
    ingestProgress,
    matchedCount,
    totalRecords,
    isLoading,
    loadError,
    cancelled,
    warnings,
  } = useEvtxMetaState();

  const eventCountDisplay = fileInfo
    ? `${fileInfo.fileName} - ${matchedCount}/${totalRecords} events`
    : "No file loaded";

  const progressDisplay = loadError
    ? "Import failed"
    : cancelled
      ? "Import cancelled"
      : isLoading
        ? `Importing ${Math.round(ingestProgress * 100)}%`
        : warnings.length
          ? "Ready with warnings"
          : "Ready";

  return (
    <Bar>
      <Item>{eventCountDisplay}</Item>
      {fileInfo && <Item>Chunks: {fileInfo.totalChunks}</Item>}
      <Item>{progressDisplay}</Item>
    </Bar>
  );
};
