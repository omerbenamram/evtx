import React, { useState, useEffect, useCallback } from "react";
import EvtxStorage from "../lib/storage";
import styled from "styled-components";
import { TreeView, type TreeNode, ContextMenu } from "./Windows";
import {
  Folder20Regular,
  FolderOpen20Filled,
  Document20Regular,
  Delete20Regular,
} from "@fluentui/react-icons";

const TreeContainer = styled.div`
  height: 100%;
  overflow-y: auto;
  background: ${({ theme }) => theme.colors.background.secondary};
  user-select: none;
`;

const TreeHeader = styled.div`
  padding: ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.md};
  font-weight: 600;
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.light};
  background: ${({ theme }) => theme.colors.background.tertiary};
`;

interface EventLogNode {
  id: string;
  label: string;
  icon?: React.ReactNode;
  expandedIcon?: React.ReactNode;
  children?: EventLogNode[];
  logPath?: string;
  description?: string;
  fileId?: string; // For cached recent logs
}

// Built-in sample log(s) shipped with the viewer. They are served from the
// /samples/ path and are *not* downloaded until the user explicitly selects
// them.
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
        description: "Built-in Windows Security log sample",
      },
    ],
  },
];

async function fetchRecentNodes(): Promise<EventLogNode[]> {
  const storage = await EvtxStorage.getInstance();
  const files = await storage.listFiles();

  // sort by pinned then lastOpened desc
  files.sort((a, b) => {
    if (a.pinned && !b.pinned) return -1;
    if (!a.pinned && b.pinned) return 1;
    return b.lastOpened - a.lastOpened;
  });

  const pinnedChildren: EventLogNode[] = [];
  const recentChildren: EventLogNode[] = [];

  files.forEach((f) => {
    const node: EventLogNode = {
      id: `recent-${f.fileId}`,
      label: f.fileName,
      icon: <Document20Regular />,
      fileId: f.fileId,
      description: `${(f.fileSize / 1024 / 1024).toFixed(1)} MB`,
    };
    if (f.pinned) pinnedChildren.push(node);
    else recentChildren.push(node);
  });

  const nodes: EventLogNode[] = [];
  if (pinnedChildren.length) {
    nodes.push({
      id: "pinned",
      label: "Pinned Logs",
      icon: <Folder20Regular />,
      expandedIcon: <FolderOpen20Filled />,
      children: pinnedChildren,
    });
  }
  if (recentChildren.length) {
    nodes.push({
      id: "recent",
      label: "Recent Logs",
      icon: <Folder20Regular />,
      expandedIcon: <FolderOpen20Filled />,
      children: recentChildren,
    });
  }
  return nodes;
}

interface FileTreeProps {
  onNodeSelect?: (node: EventLogNode) => void;
  selectedNodeId?: string;
  activeFileId?: string | null; // file currently ingesting
  ingestProgress?: number; // 0..1
  refreshVersion?: number; // internal, bump to force recent list update
}

export const FileTree: React.FC<FileTreeProps> = ({
  onNodeSelect,
  selectedNodeId,
  activeFileId,
  ingestProgress = 1,
  refreshVersion = 0,
}) => {
  const [treeData, setTreeData] = useState<EventLogNode[]>(baseStructure);

  useEffect(() => {
    // Load recent logs once component mounts
    (async () => {
      const recent = await fetchRecentNodes();
      setTreeData([...recent, ...baseStructure]);
    })();
  }, [refreshVersion]);

  // refresh helper (e.g., after load) – exposed via context could be nicer
  const refreshRecent = useCallback(async () => {
    const recent = await fetchRecentNodes();
    setTreeData([...recent, ...baseStructure]);
  }, []);

  const convertToTreeNodes = (nodes: EventLogNode[]): TreeNode[] => {
    return nodes.map((node) => {
      const showPct =
        node.fileId &&
        activeFileId &&
        node.fileId === activeFileId &&
        ingestProgress < 1;
      const pctRaw = ingestProgress * 100;
      const pctDisplay = pctRaw < 0.01 ? 0.01 : Math.round(pctRaw * 100) / 100; // two decimals
      const labelWithPct = showPct
        ? `${node.label} (${pctDisplay.toFixed(2)}%)`
        : node.label;

      return {
        id: node.id,
        label: labelWithPct,
        icon: node.icon,
        expandedIcon: node.expandedIcon,
        children: node.children ? convertToTreeNodes(node.children) : undefined,
        data: node,
      };
    });
  };

  const handleSelect = (treeNode: TreeNode) => {
    const nodeId = treeNode.id;

    const findNode = (nodes: EventLogNode[]): EventLogNode | null => {
      for (const node of nodes) {
        if (node.id === nodeId) return node;
        if (node.children) {
          const found = findNode(node.children);
          if (found) return found;
        }
      }
      return null;
    };

    const selectedNode = findNode(treeData);
    if (selectedNode) {
      if (onNodeSelect) onNodeSelect(selectedNode);
      // If a recent log was opened, bump lastOpened and refresh tree
      if (selectedNode.fileId) {
        refreshRecent();
      }
    }
  };

  // -----------------------------
  // Context menu (right-click)
  // -----------------------------

  const [menuState, setMenuState] = useState<{
    x: number;
    y: number;
    target: EventLogNode;
  } | null>(null);

  const handleContextMenu = (treeNode: TreeNode, e: React.MouseEvent) => {
    const dataNode = treeNode.data as EventLogNode | undefined;
    if (!dataNode?.fileId) return; // Only for cached files

    e.preventDefault();
    setMenuState({ x: e.clientX, y: e.clientY, target: dataNode });
  };

  const closeMenu = useCallback(() => setMenuState(null), []);

  const handleDelete = useCallback(async () => {
    if (!menuState) return;
    const storage = await EvtxStorage.getInstance();
    await storage.deleteFile(menuState.target.fileId!);
    await refreshRecent();
    closeMenu();
  }, [menuState, refreshRecent, closeMenu]);

  return (
    <TreeContainer>
      <TreeHeader>Event Logs</TreeHeader>
      <TreeView
        nodes={convertToTreeNodes(treeData)}
        selectedNodeId={selectedNodeId}
        onNodeClick={handleSelect}
        onNodeContextMenu={handleContextMenu}
        showLines={false}
        defaultExpanded={[]}
      />

      {menuState && (
        <ContextMenu
          position={{ x: menuState.x, y: menuState.y }}
          onClose={closeMenu}
          items={[
            {
              id: "delete",
              label: "Delete",
              icon: <Delete20Regular />,
              onClick: handleDelete,
            },
          ]}
        />
      )}
    </TreeContainer>
  );
};
