import React, { useState, useEffect, useCallback } from "react";
import EvtxStorage from "../lib/storage";
import { styled } from "styled-components";
import { TreeView, type TreeNode, ContextMenu, SidebarHeader } from "./Windows";
import { errorMessage } from "../lib/types";
import {
  Folder20Regular,
  FolderOpen20Filled,
  Document20Regular,
  Delete20Regular,
} from "@fluentui/react-icons";

const TreeContainer = styled.div`
  height: 100%;
  min-height: 0;
  min-width: 0;
  overflow-y: auto;
  background: ${({ theme }) => theme.colors.background.secondary};
  user-select: none;
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
    icon: <Folder20Regular />,
    expandedIcon: <FolderOpen20Filled />,
    children: [
      {
        id: "sample-security",
        label: "security.evtx (sample)",
        icon: <Document20Regular />,
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
      icon: <Folder20Regular />,
      expandedIcon: <FolderOpen20Filled />,
      children: files.map((f) => ({
        id: `recent-${f.fileId}`,
        label: f.fileName,
        icon: <Document20Regular />,
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

  const convertToTreeNodes = (nodes: EventLogNode[]): TreeNode[] =>
    nodes.map((node) => ({
      id: node.id,
      label:
        node.fileId && node.fileId === activeFileId && ingestProgress < 1
          ? `${node.label} (${Math.max(0.01, ingestProgress * 100).toFixed(2)}%)`
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
      <SidebarHeader>Event Logs</SidebarHeader>
      {error && (
        <p role="alert" style={{ padding: 12 }}>
          {error}
        </p>
      )}
      <TreeView
        nodes={convertToTreeNodes(treeData)}
        selectedNodeId={selectedNodeId}
        onNodeClick={handleSelect}
        onNodeContextMenu={handleContextMenu}
        defaultExpanded={["examples", "recent"]}
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
              icon: <Delete20Regular />,
              onClick: () => void handleDelete(),
            },
          ]}
        />
      )}
    </TreeContainer>
  );
};
