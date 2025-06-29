import React, { useState } from "react";
import styled from "styled-components";
import { TreeView, type TreeNode } from "./Windows";
import {
  Folder20Regular,
  FolderOpen20Filled,
  Document20Regular,
  Warning20Regular,
  Apps20Regular,
  Shield20Regular,
  Settings20Regular,
  WindowDevTools20Regular,
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
}

const eventLogStructure: EventLogNode[] = [
  {
    id: "event-viewer",
    label: "Event Viewer (Local)",
    icon: <WindowDevTools20Regular />,
    expandedIcon: <WindowDevTools20Regular />,
    children: [
      {
        id: "custom-views",
        label: "Custom Views",
        icon: <Folder20Regular />,
        expandedIcon: <FolderOpen20Filled />,
        children: [
          {
            id: "administrative-events",
            label: "Administrative Events",
            icon: <Warning20Regular />,
            description:
              "Critical, Error, and Warning events from all administrative logs",
          },
        ],
      },
      {
        id: "windows-logs",
        label: "Windows Logs",
        icon: <Folder20Regular />,
        expandedIcon: <FolderOpen20Filled />,
        children: [
          {
            id: "application",
            label: "Application",
            icon: <Apps20Regular />,
            logPath: "Application.evtx",
            description: "Events logged by applications",
          },
          {
            id: "security",
            label: "Security",
            icon: <Shield20Regular />,
            logPath: "Security.evtx",
            description: "Security audit events",
          },
          {
            id: "setup",
            label: "Setup",
            icon: <Settings20Regular />,
            logPath: "Setup.evtx",
            description: "Events related to application setup",
          },
          {
            id: "system",
            label: "System",
            icon: <WindowDevTools20Regular />,
            logPath: "System.evtx",
            description: "Events logged by Windows system components",
          },
          {
            id: "forwarded-events",
            label: "Forwarded Events",
            icon: <Document20Regular />,
            logPath: "ForwardedEvents.evtx",
            description: "Events forwarded from other computers",
          },
        ],
      },
      {
        id: "applications-services",
        label: "Applications and Services Logs",
        icon: <Folder20Regular />,
        expandedIcon: <FolderOpen20Filled />,
        children: [
          {
            id: "hardware-events",
            label: "Hardware Events",
            icon: <Document20Regular />,
          },
          {
            id: "internet-explorer",
            label: "Internet Explorer",
            icon: <Document20Regular />,
          },
          {
            id: "key-management",
            label: "Key Management Service",
            icon: <Document20Regular />,
          },
          {
            id: "microsoft",
            label: "Microsoft",
            icon: <Folder20Regular />,
            expandedIcon: <FolderOpen20Filled />,
            children: [
              {
                id: "windows",
                label: "Windows",
                icon: <Folder20Regular />,
                expandedIcon: <FolderOpen20Filled />,
                children: [
                  {
                    id: "powershell",
                    label: "PowerShell",
                    icon: <Document20Regular />,
                    logPath: "Microsoft-Windows-PowerShell%4Operational.evtx",
                  },
                  {
                    id: "windows-defender",
                    label: "Windows Defender",
                    icon: <Shield20Regular />,
                    logPath:
                      "Microsoft-Windows-Windows Defender%4Operational.evtx",
                  },
                ],
              },
            ],
          },
        ],
      },
      {
        id: "saved-logs",
        label: "Saved Logs",
        icon: <Folder20Regular />,
        expandedIcon: <FolderOpen20Filled />,
        children: [],
      },
    ],
  },
];

interface FileTreeProps {
  onNodeSelect?: (node: EventLogNode) => void;
  selectedNodeId?: string;
}

export const FileTree: React.FC<FileTreeProps> = ({
  onNodeSelect,
  selectedNodeId,
}) => {
  const [treeData] = useState(eventLogStructure);

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
    if (selectedNode && onNodeSelect) {
      onNodeSelect(selectedNode);
    }
  };

  return (
    <TreeContainer>
      <TreeHeader>Event Logs</TreeHeader>
      <TreeView
        nodes={convertToTreeNodes(treeData)}
        selectedNodeId={selectedNodeId}
        onNodeClick={handleSelect}
        showLines={false}
        defaultExpanded={["event-viewer", "windows-logs"]}
      />
    </TreeContainer>
  );
};
