import React, { useState, useCallback, useEffect, useRef } from "react";
import styled, { keyframes } from "styled-components";
import { CloudArrowUp48Regular } from "@fluentui/react-icons";

const fadeIn = keyframes`
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
`;

const scaleIn = keyframes`
  from {
    transform: scale(0.8);
  }
  to {
    transform: scale(1);
  }
`;

const Overlay = styled.div<{ $isVisible: boolean }>`
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(255, 255, 255, 0.95);
  backdrop-filter: blur(4px);
  display: ${(props) => (props.$isVisible ? "flex" : "none")};
  align-items: center;
  justify-content: center;
  z-index: 1000;
  animation: ${fadeIn} 200ms ease-out;
`;

const DropZone = styled.div<{ $isDragOver: boolean }>`
  width: 400px;
  height: 300px;
  border: 3px dashed
    ${({ $isDragOver, theme }) =>
      $isDragOver ? theme.colors.accent.primary : theme.colors.border.medium};
  border-radius: ${({ theme }) => theme.borderRadius.lg};
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: ${({ theme }) => theme.spacing.lg};
  background: ${({ $isDragOver, theme }) =>
    $isDragOver
      ? theme.colors.selection.background
      : theme.colors.background.secondary};
  transition: all ${({ theme }) => theme.transitions.normal};
  animation: ${scaleIn} 200ms ease-out;

  &:hover {
    border-color: ${({ theme }) => theme.colors.accent.primary};
    background: ${({ theme }) => theme.colors.selection.background};
  }
`;

const IconWrapper = styled.div<{ $isDragOver: boolean }>`
  color: ${({ $isDragOver, theme }) =>
    $isDragOver ? theme.colors.accent.primary : theme.colors.text.secondary};
  transition: all ${({ theme }) => theme.transitions.normal};
  transform: ${({ $isDragOver }) => ($isDragOver ? "scale(1.1)" : "scale(1)")};
`;

const Title = styled.h2`
  font-size: ${({ theme }) => theme.fontSize.title};
  color: ${({ theme }) => theme.colors.text.primary};
  margin: 0;
`;

const Subtitle = styled.p`
  font-size: ${({ theme }) => theme.fontSize.body};
  color: ${({ theme }) => theme.colors.text.secondary};
  margin: 0;
`;

const FileInput = styled.input`
  display: none;
`;

interface DragDropOverlayProps {
  onFileSelect: (file: File) => void;
  acceptedExtensions?: string[];
}

export const DragDropOverlay: React.FC<DragDropOverlayProps> = ({
  onFileSelect,
  acceptedExtensions = [".evtx"],
}) => {
  const [isVisible, setIsVisible] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const dragCounter = useRef(0);

  const handleDragEnter = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    if (e.dataTransfer?.items && e.dataTransfer.items.length > 0) {
      dragCounter.current += 1;
      setIsVisible(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    dragCounter.current -= 1;
    if (dragCounter.current === 0) {
      setIsVisible(false);
      setIsDragOver(false);
    }
  }, []);

  const handleDragOver = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleDrop = useCallback(
    (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();

      setIsVisible(false);
      setIsDragOver(false);
      dragCounter.current = 0;

      const files = e.dataTransfer?.files;
      if (files && files.length > 0) {
        const file = files[0];
        const extension = "." + file.name.split(".").pop()?.toLowerCase();

        if (acceptedExtensions.includes(extension)) {
          onFileSelect(file);
        } else {
          alert(
            `Please select a valid file type: ${acceptedExtensions.join(", ")}`
          );
        }
      }
    },
    [acceptedExtensions, onFileSelect]
  );

  const handleOverlayDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(true);
  }, []);

  const handleOverlayDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();

    // Only set isDragOver to false if we're leaving the drop zone
    const relatedTarget = e.relatedTarget as HTMLElement;
    if (!relatedTarget || !e.currentTarget.contains(relatedTarget)) {
      setIsDragOver(false);
    }
  }, []);

  const handleFileInputChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = e.target.files;
      if (files && files.length > 0) {
        onFileSelect(files[0]);
      }
    },
    [onFileSelect]
  );

  const handleClick = useCallback(() => {
    const input = document.getElementById("file-input") as HTMLInputElement;
    input?.click();
  }, []);

  useEffect(() => {
    document.addEventListener("dragenter", handleDragEnter);
    document.addEventListener("dragleave", handleDragLeave);
    document.addEventListener("dragover", handleDragOver);
    document.addEventListener("drop", handleDrop);

    return () => {
      document.removeEventListener("dragenter", handleDragEnter);
      document.removeEventListener("dragleave", handleDragLeave);
      document.removeEventListener("dragover", handleDragOver);
      document.removeEventListener("drop", handleDrop);
    };
  }, [handleDragEnter, handleDragLeave, handleDragOver, handleDrop]);

  return (
    <Overlay $isVisible={isVisible}>
      <DropZone
        $isDragOver={isDragOver}
        onDragEnter={handleOverlayDragEnter}
        onDragLeave={handleOverlayDragLeave}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault();
          handleDrop(e.nativeEvent);
        }}
        onClick={handleClick}
      >
        <IconWrapper $isDragOver={isDragOver}>
          <CloudArrowUp48Regular />
        </IconWrapper>
        <Title>Drop EVTX file here</Title>
        <Subtitle>or click to browse</Subtitle>
        <FileInput
          id="file-input"
          type="file"
          accept={acceptedExtensions.join(",")}
          onChange={handleFileInputChange}
        />
      </DropZone>
    </Overlay>
  );
};
