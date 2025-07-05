import React, { useState, useCallback } from "react";
import styled, { css } from "styled-components";

export interface TreeNode {
  id: string;
  label: string;
  icon?: React.ReactNode;
  /** Optional icon to display when node is expanded (if different). */
  expandedIcon?: React.ReactNode;
  children?: TreeNode[];
  isExpanded?: boolean;
  isSelected?: boolean;
  onClick?: () => void;
  /** Arbitrary metadata associated with the node (not used by TreeView itself). */
  data?: unknown;
}

export interface TreeViewProps {
  nodes: TreeNode[];
  onNodeClick?: (node: TreeNode) => void;
  /** Fired when a contextmenu (right-click) event occurs on a node. */
  onNodeContextMenu?: (node: TreeNode, event: React.MouseEvent) => void;
  onNodeExpand?: (node: TreeNode, isExpanded: boolean) => void;
  selectedNodeId?: string;
  expandedNodeIds?: Set<string>;
  showLines?: boolean;
  /** ids of nodes that should be expanded initially (uncontrolled mode) */
  defaultExpanded?: string[];
}

const TreeContainer = styled.div`
  font-family: ${({ theme }) => theme.fonts.body};
  font-size: ${({ theme }) => theme.fontSize.body};
  color: ${({ theme }) => theme.colors.text.primary};
  user-select: none;
`;

const TreeNodeContainer = styled.div<{ $level: number; $showLines?: boolean }>`
  position: relative;

  ${({ $level, $showLines, theme }) =>
    $showLines &&
    $level > 0 &&
    css`
      &::before {
        content: "";
        position: absolute;
        left: ${($level - 1) * 20 + 10}px;
        top: 0;
        bottom: 0;
        width: 1px;
        background-color: ${theme.colors.border.light};
      }
    `}
`;

const TreeNodeContent = styled.div<{ $isSelected?: boolean; $level: number }>`
  display: flex;
  align-items: center;
  padding: 4px 8px;
  padding-left: ${({ $level }) => $level * 20 + 8}px;
  cursor: pointer;
  border-radius: ${({ theme }) => theme.borderRadius.sm};
  transition: all ${({ theme }) => theme.transitions.fast};

  &:hover {
    background-color: ${({ theme }) => theme.colors.background.hover};
  }

  ${({ $isSelected, $level, theme }) =>
    $isSelected &&
    css`
      background-color: ${theme.colors.selection.background};
      border: 1px solid ${theme.colors.selection.border};
      padding: 3px 7px;
      padding-left: ${$level * 20 + 7}px;
    `}
`;

const ExpandIcon = styled.span<{ $isExpanded: boolean }>`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  margin-right: 4px;
  transition: transform ${({ theme }) => theme.transitions.fast};

  ${({ $isExpanded }) =>
    $isExpanded &&
    css`
      transform: rotate(90deg);
    `}

  &::before {
    content: "▶";
    font-size: 10px;
    color: ${({ theme }) => theme.colors.text.secondary};
  }
`;

const EmptySpace = styled.span`
  display: inline-block;
  width: 20px;
`;

const NodeIcon = styled.span`
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  margin-right: 6px;
  color: ${({ theme }) => theme.colors.text.secondary};
`;

const NodeLabel = styled.span`
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
`;

interface TreeNodeComponentProps {
  node: TreeNode;
  level: number;
  onNodeClick?: (node: TreeNode) => void;
  /** Fired when a contextmenu (right-click) event occurs on a node. */
  onNodeContextMenu?: (node: TreeNode, event: React.MouseEvent) => void;
  onNodeExpand?: (node: TreeNode, isExpanded: boolean) => void;
  selectedNodeId?: string;
  expandedNodeIds: Set<string>;
  showLines?: boolean;
}

const TreeNodeComponent: React.FC<TreeNodeComponentProps> = ({
  node,
  level,
  onNodeClick,
  onNodeContextMenu,
  onNodeExpand,
  selectedNodeId,
  expandedNodeIds,
  showLines,
}) => {
  const isExpanded = expandedNodeIds.has(node.id);
  const isSelected = selectedNodeId === node.id;
  const hasChildren = node.children && node.children.length > 0;

  const handleClick = useCallback(() => {
    if (node.onClick) {
      node.onClick();
    }
    if (onNodeClick) {
      onNodeClick(node);
    }
  }, [node, onNodeClick]);

  const handleContextMenu = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      if (onNodeContextMenu) {
        onNodeContextMenu(node, e);
      }
    },
    [node, onNodeContextMenu]
  );

  const handleExpand = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      if (hasChildren && onNodeExpand) {
        onNodeExpand(node, !isExpanded);
      }
    },
    [node, isExpanded, hasChildren, onNodeExpand]
  );

  return (
    <TreeNodeContainer $level={level} $showLines={showLines}>
      <TreeNodeContent
        $isSelected={isSelected}
        $level={level}
        onClick={handleClick}
        onContextMenu={handleContextMenu}
      >
        {hasChildren ? (
          <ExpandIcon $isExpanded={isExpanded} onClick={handleExpand} />
        ) : (
          <EmptySpace />
        )}
        {node.icon && (
          <NodeIcon>
            {isExpanded && node.expandedIcon ? node.expandedIcon : node.icon}
          </NodeIcon>
        )}
        <NodeLabel>{node.label}</NodeLabel>
      </TreeNodeContent>
      {hasChildren && isExpanded && (
        <div>
          {node.children!.map((childNode) => (
            <TreeNodeComponent
              key={childNode.id}
              node={childNode}
              level={level + 1}
              onNodeClick={onNodeClick}
              onNodeContextMenu={onNodeContextMenu}
              onNodeExpand={onNodeExpand}
              selectedNodeId={selectedNodeId}
              expandedNodeIds={expandedNodeIds}
              showLines={showLines}
            />
          ))}
        </div>
      )}
    </TreeNodeContainer>
  );
};

export const TreeView: React.FC<TreeViewProps> = ({
  nodes,
  onNodeClick,
  onNodeContextMenu,
  onNodeExpand,
  selectedNodeId,
  expandedNodeIds: providedExpandedNodeIds,
  showLines = false,
  defaultExpanded = [],
}) => {
  const [internalExpandedNodeIds, setInternalExpandedNodeIds] = useState<
    Set<string>
  >(() => new Set(defaultExpanded));

  const expandedNodeIds = providedExpandedNodeIds || internalExpandedNodeIds;

  const handleNodeExpand = useCallback(
    (node: TreeNode, isExpanded: boolean) => {
      if (onNodeExpand) {
        onNodeExpand(node, isExpanded);
      } else {
        setInternalExpandedNodeIds((prev) => {
          const newSet = new Set(prev);
          if (isExpanded) {
            newSet.add(node.id);
          } else {
            newSet.delete(node.id);
          }
          return newSet;
        });
      }
    },
    [onNodeExpand]
  );

  return (
    <TreeContainer>
      {nodes.map((node) => (
        <TreeNodeComponent
          key={node.id}
          node={node}
          level={0}
          onNodeClick={onNodeClick}
          onNodeContextMenu={onNodeContextMenu}
          onNodeExpand={handleNodeExpand}
          selectedNodeId={selectedNodeId}
          expandedNodeIds={expandedNodeIds}
          showLines={showLines}
        />
      ))}
    </TreeContainer>
  );
};
