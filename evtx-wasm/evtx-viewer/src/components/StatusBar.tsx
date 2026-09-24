import React from "react";
import { styled } from "styled-components";
import { useEvtxMetaState } from "../state/store";
import { ProgressBar, Tooltip } from "./Windows";

const Bar = styled.footer`
  display: flex;
  align-items: center;
  flex-shrink: 0;
  gap: 8px;
  height: ${({ theme }) => theme.size.row};
  padding: 0 ${({ theme }) => theme.spacing.sm};
  overflow: hidden;
  background: ${({ theme }) => theme.colors.surface.base};
  border-top: 1px solid ${({ theme }) => theme.colors.stroke.divider};
  color: ${({ theme }) => theme.colors.text.secondary};
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
`;

const Text = styled.span`
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
`;

const Progress = styled(ProgressBar)`
  flex: 0 0 96px;
`;

const Cancel = styled.button`
  flex-shrink: 0;
  height: 18px;
  padding: 0 6px;
  border: 0;
  border-radius: ${({ theme }) => theme.radius.control};
  background: transparent;
  color: ${({ theme }) => theme.colors.accent.rest};
  cursor: default;
  &:hover {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
`;

const count = (value: number) => value.toLocaleString();

export const StatusBar: React.FC<{ onCancel: () => void }> = ({ onCancel }) => {
  const {
    fileInfo,
    ingestProgress,
    matchedCount,
    totalRecords,
    isLoading,
    loadingMessage,
    loadError,
    cancelled,
    warnings,
  } = useEvtxMetaState();

  const status = loadError
    ? "Import failed"
    : cancelled
      ? "Import cancelled"
      : isLoading
        ? ingestProgress > 0 && ingestProgress < 1
          ? `Importing ${Math.floor(ingestProgress * 100)}%`
          : loadingMessage || "Opening log…"
        : !fileInfo
          ? "No log open"
          : warnings.length
            ? "Ready with warnings"
            : "Ready";
  const counts = fileInfo ? `${count(totalRecords)} events · ${count(matchedCount)} shown · ` : "";

  const text = (
    <Text>
      {counts}
      {status}
    </Text>
  );
  return (
    <Bar>
      {fileInfo ? (
        <Tooltip label={`${fileInfo.fileName} · ${count(fileInfo.totalChunks)} chunks`}>
          {text}
        </Tooltip>
      ) : (
        text
      )}
      {isLoading && (
        <>
          <Progress value={ingestProgress} />
          {ingestProgress < 1 && (
            <Cancel type="button" onClick={onCancel}>
              Cancel
            </Cancel>
          )}
        </>
      )}
    </Bar>
  );
};
