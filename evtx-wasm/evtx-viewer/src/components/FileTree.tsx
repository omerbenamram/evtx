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
import { theme } from "../styles/theme";

const TreeContainer = styled.div`
  height: 100%;
  overflow-y: auto;
  background: ${theme.colors.background.secondary};
  user-select: none;
`;

const TreeHeader = styled.div`
  padding: ${theme.spacing.sm} ${theme.spacing.md};
  font-weight: 600;
  border-bottom: 1px solid ${theme.colors.border.light};
  background: ${theme.colors.background.tertiary};
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

// Removed the placeholder “Event Viewer (Local)” hierarchy – it did not provide any functionality.
const baseStructure: EventLogNode[] = [];

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
}

export const FileTree: React.FC<FileTreeProps> = ({
  onNodeSelect,
  selectedNodeId,
}) => {
  const [treeData, setTreeData] = useState<EventLogNode[]>(baseStructure);

  useEffect(() => {
    // Load recent logs once component mounts
    (async () => {
      const recent = await fetchRecentNodes();
      setTreeData([...recent, ...baseStructure]);
    })();
  }, []);

  // refresh helper (e.g., after load) – exposed via context could be nicer
  const refreshRecent = useCallback(async () => {
    const recent = await fetchRecentNodes();
    setTreeData([...recent, ...baseStructure]);
  }, []);

  const convertToTreeNodes = (nodes: EventLogNode[]): TreeNode[] => {
    return nodes.map((node) => ({
      id: node.id,
      label: node.label,
      icon: node.icon,
      expandedIcon: node.expandedIcon,
      children: node.children ? convertToTreeNodes(node.children) : undefined,
      data: node,
    }));
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
