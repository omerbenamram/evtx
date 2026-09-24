import React, { useState, useEffect, useCallback } from "react";
import EvtxStorage from "../lib/storage";
import { styled } from "styled-components";
import { TreeView, type TreeNode, ContextMenu } from "./Windows";
import { errorMessage } from "../lib/types";
import {
  Folder16Regular,
  FolderOpen16Regular,
  DocumentBulletList16Regular,
  Delete16Regular,
  Desktop16Regular,
} from "@fluentui/react-icons";

const TreeContainer = styled.div`
  height: 100%;
  min-height: 0;
  min-width: 0;
  overflow-y: auto;
  padding-top: 2px;
  background: ${({ theme }) => theme.colors.surface.pane};
  user-select: none;
`;
const ErrorText = styled.p`
  padding: 4px 8px;
  color: ${({ theme }) => theme.colors.severity.error};
`;

interface EventLogNode {
  id: string;
  label: string;
  icon?: React.ReactNode;
  expandedIcon?: React.ReactNode;
  children?: EventLogNode[];
  logPath?: string;
  fileId?: string;
}

// Fetched from /samples/ only when selected.
const baseStructure: EventLogNode[] = [
  {
    id: "examples",
    label: "Example Logs",
    icon: <Folder16Regular />,
    expandedIcon: <FolderOpen16Regular />,
    children: [
      {
        id: "sample-security",
        label: "security.evtx (sample)",
        icon: <DocumentBulletList16Regular />,
        logPath: "samples/security.evtx",
      },
    ],
  },
];

async function fetchRecentNodes(): Promise<EventLogNode[]> {
  const storage = await EvtxStorage.getInstance();
  const files = (await storage.listFiles()).toSorted((a, b) => b.lastOpened - a.lastOpened);
  if (!files.length) return [];
  return [
    {
      id: "recent",
      label: "Recent Logs",
      icon: <Folder16Regular />,
      expandedIcon: <FolderOpen16Regular />,
      children: files.map((f) => ({
        id: `recent-${f.fileId}`,
        label: f.fileName,
        icon: <DocumentBulletList16Regular />,
        fileId: f.fileId,
      })),
    },
  ];
}

interface FileTreeProps {
  onNodeSelect: (node: EventLogNode) => void;
  selectedNodeId: string;
  activeFileId: string | null;
  ingestProgress: number;
  /** Bump to re-read Recent Logs. */
  refreshVersion: number;
}

export const FileTree: React.FC<FileTreeProps> = ({
  onNodeSelect,
  selectedNodeId,
  activeFileId,
  ingestProgress,
  refreshVersion,
}) => {
  const [treeData, setTreeData] = useState<EventLogNode[]>(baseStructure);
  const [error, setError] = useState("");

  const [refresh, setRefresh] = useState(0);

  useEffect(() => {
    let cancelled = false;
    fetchRecentNodes()
      .then((recent) => {
        if (!cancelled) {
          setTreeData([...recent, ...baseStructure]);
          setError("");
        }
      })
      .catch((cause: unknown) => {
        if (!cancelled) setError(`Could not read recent logs: ${errorMessage(cause)}`);
      });
    return () => {
      cancelled = true;
    };
  }, [refreshVersion, refresh]);

  const root: EventLogNode = {
    id: "root",
    label: "Event Viewer (Local)",
    icon: <Desktop16Regular />,
    children: treeData,
  };

  const convertToTreeNodes = (nodes: EventLogNode[]): TreeNode[] =>
    nodes.map((node) => ({
      id: node.id,
      label:
        node.fileId && node.fileId === activeFileId && ingestProgress < 1
          ? `${node.label} (${Math.floor(ingestProgress * 100)}%)`
          : node.label,
      icon: node.icon,
      expandedIcon: node.expandedIcon,
      children: node.children ? convertToTreeNodes(node.children) : undefined,
    }));

  function findNode(id: string, nodes = treeData): EventLogNode | undefined {
    for (const node of nodes) {
      if (node.id === id) return node;
      const child = node.children && findNode(id, node.children);
      if (child) return child;
    }
    return undefined;
  }

  function handleSelect(treeNode: TreeNode) {
    const node = findNode(treeNode.id);
    if (node) onNodeSelect(node);
  }

  const [menuState, setMenuState] = useState<{
    x: number;
    y: number;
    fileId: string;
    returnFocus: HTMLElement;
  } | null>(null);

  function handleContextMenu(treeNode: TreeNode, event: React.MouseEvent<HTMLElement>) {
    const node = findNode(treeNode.id);
    if (node?.fileId) {
      const bounds = event.currentTarget.getBoundingClientRect();
      setMenuState({
        x: event.clientX || bounds.left,
        y: event.clientY || bounds.bottom,
        fileId: node.fileId,
        returnFocus: event.currentTarget,
      });
    }
  }

  const closeMenu = useCallback(() => setMenuState(null), []);

  const handleDelete = useCallback(async () => {
    if (!menuState) return;
    try {
      const storage = await EvtxStorage.getInstance();
      await storage.deleteFile(menuState.fileId);
      setRefresh((value) => value + 1);
    } catch (cause) {
      setError(`Could not remove the cached log: ${errorMessage(cause)}`);
    }
  }, [menuState]);

  return (
    <TreeContainer>
      {error && <ErrorText role="alert">{error}</ErrorText>}
      <TreeView
        nodes={convertToTreeNodes([root])}
        selectedNodeId={selectedNodeId}
        onNodeClick={handleSelect}
        onNodeContextMenu={handleContextMenu}
        defaultExpanded={["root", "examples", "recent"]}
      />

      {menuState && (
        <ContextMenu
          position={{ x: menuState.x, y: menuState.y }}
          onClose={closeMenu}
          returnFocus={menuState.returnFocus}
          ariaLabel="Cached log actions"
          items={[
            {
              id: "delete",
              label: "Remove from recent logs",
              icon: <Delete16Regular />,
              onClick: () => void handleDelete(),
            },
          ]}
        />
      )}
    </TreeContainer>
  );
};
