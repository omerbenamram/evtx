import React, { useState, useEffect } from "react";
import { styled } from "styled-components";
import { CloudArrowUp48Regular } from "@fluentui/react-icons";

const Overlay = styled.div<{ $isVisible: boolean }>`
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: ${({ theme }) => theme.colors.surface.base};
  display: ${(props) => (props.$isVisible ? "flex" : "none")};
  align-items: center;
  justify-content: center;
  z-index: 1000;
`;

const DropZone = styled.div<{ $isDragOver: boolean }>`
  width: min(400px, calc(100vw - 32px));
  height: 300px;
  border: 1px dashed
    ${({ $isDragOver, theme }) =>
      $isDragOver ? theme.colors.accent.rest : theme.colors.stroke.control};
  border-radius: ${({ theme }) => theme.radius.flyout};
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: ${({ theme }) => theme.spacing.lg};
  background: ${({ $isDragOver, theme }) =>
    $isDragOver ? theme.colors.fill.selected : theme.colors.surface.pane};
`;

const IconWrapper = styled.div<{ $isDragOver: boolean }>`
  color: ${({ $isDragOver, theme }) =>
    $isDragOver ? theme.colors.accent.rest : theme.colors.text.secondary};
`;

const Title = styled.h2`
  font-weight: 600;
  font-size: ${({ theme }) => theme.fontSize.title};
  color: ${({ theme }) => theme.colors.text.primary};
  margin: 0;
`;

const FileInput = styled.input`
  display: none;
`;

const EXTENSION = ".evtx";

export const DragDropOverlay: React.FC<{ onFileSelect: (file: File) => void }> = ({
  onFileSelect,
}) => {
  const [isVisible, setIsVisible] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);

  useEffect(() => {
    let depth = 0;
    const hide = () => {
      depth = 0;
      setIsVisible(false);
      setIsDragOver(false);
    };
    const onDragEnter = (e: DragEvent) => {
      e.preventDefault();
      if (!e.dataTransfer?.items.length) return;
      depth++;
      setIsVisible(true);
    };
    const onDragLeave = (e: DragEvent) => {
      e.preventDefault();
      if (--depth <= 0) hide();
    };
    const onDragOver = (e: DragEvent) => e.preventDefault();
    const onDrop = (e: DragEvent) => {
      e.preventDefault();
      hide();
      const file = e.dataTransfer?.files[0];
      if (!file) return;
      if (file.name.toLowerCase().endsWith(EXTENSION)) onFileSelect(file);
      else alert(`Please select a valid file type: ${EXTENSION}`);
    };
    document.addEventListener("dragenter", onDragEnter);
    document.addEventListener("dragleave", onDragLeave);
    document.addEventListener("dragover", onDragOver);
    document.addEventListener("drop", onDrop);
    return () => {
      document.removeEventListener("dragenter", onDragEnter);
      document.removeEventListener("dragleave", onDragLeave);
      document.removeEventListener("dragover", onDragOver);
      document.removeEventListener("drop", onDrop);
    };
  }, [onFileSelect]);

  return (
    <Overlay $isVisible={isVisible}>
      <DropZone
        $isDragOver={isDragOver}
        onDragEnter={() => setIsDragOver(true)}
        onDragLeave={(e) => {
          if (!(e.relatedTarget instanceof Node) || !e.currentTarget.contains(e.relatedTarget))
            setIsDragOver(false);
        }}
      >
        <IconWrapper $isDragOver={isDragOver}>
          <CloudArrowUp48Regular />
        </IconWrapper>
        <Title>Drop EVTX file here</Title>
        <FileInput
          id="file-input"
          type="file"
          accept={EXTENSION}
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (file) onFileSelect(file);
            e.target.value = "";
          }}
        />
      </DropZone>
    </Overlay>
  );
};
