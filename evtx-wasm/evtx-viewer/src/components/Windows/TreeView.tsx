import { useState, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { styled } from "styled-components";
import { ChevronDown16Regular, ChevronRight16Regular } from "@fluentui/react-icons";

export interface TreeNode {
  id: string;
  label: string;
  icon?: ReactNode;
  expandedIcon?: ReactNode;
  children?: TreeNode[];
}
export interface TreeViewProps {
  nodes: TreeNode[];
  onNodeClick?: (node: TreeNode) => void;
  onNodeContextMenu?: (node: TreeNode, event: MouseEvent<HTMLElement>) => void;
  selectedNodeId?: string;
  defaultExpanded?: string[];
}

const Tree = styled.div`
  min-width: 0;
  color: ${({ theme }) => theme.colors.text.primary};
  user-select: none;
`;
// Event Viewer tree: 22px rows, 16px indent per level; selected = light accent fill + 1px outline.
const Row = styled.div<{ $level: number; $selected: boolean }>`
  display: flex;
  align-items: center;
  gap: 4px;
  height: ${({ theme }) => theme.size.row};
  padding: 0 8px 0 ${({ $level }) => $level * 16 + 4}px;
  background: ${({ theme, $selected }) => ($selected ? theme.colors.fill.selected : "transparent")};
  box-shadow: ${({ theme, $selected }) =>
    $selected ? `inset 0 0 0 1px ${theme.colors.accent.rest}` : "none"};
  cursor: default;
  &:hover {
    background: ${({ theme, $selected }) =>
      $selected ? theme.colors.fill.selected : theme.colors.fill.hover};
  }
  &:focus-visible {
    outline-offset: -2px;
  }
  svg {
    flex-shrink: 0;
  }
  @media (forced-colors: active) {
    ${({ $selected }) => ($selected ? "outline: 1px solid Highlight; outline-offset: -1px;" : "")}
  }
`;
const Disclosure = styled.span`
  display: inline-flex;
  width: 16px;
  flex-shrink: 0;
  color: ${({ theme }) => theme.colors.text.secondary};
`;
const Label = styled.span`
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

export function TreeView({
  nodes,
  onNodeClick,
  onNodeContextMenu,
  selectedNodeId,
  defaultExpanded = [],
}: TreeViewProps) {
  const [expanded, setExpanded] = useState(() => new Set(defaultExpanded));
  const [focused, setFocused] = useState(nodes[0]?.id);
  function expand(id: string, value: boolean) {
    setExpanded((previous) => {
      const next = new Set(previous);
      if (value) next.add(id);
      else next.delete(id);
      return next;
    });
  }
  function keyDown(event: KeyboardEvent<HTMLDivElement>, node: TreeNode, parent?: string) {
    const tree = event.currentTarget.closest('[role="tree"]');
    const rows = Array.from(tree?.querySelectorAll<HTMLElement>('[role="treeitem"]') ?? []);
    const index = rows.indexOf(event.currentTarget);
    let next = index;
    if (event.key === "ArrowDown") next = Math.min(rows.length - 1, index + 1);
    else if (event.key === "ArrowUp") next = Math.max(0, index - 1);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = rows.length - 1;
    else if (event.key === "ArrowRight" && node.children?.length) {
      if (expanded.has(node.id)) next = Math.min(rows.length - 1, index + 1);
      else expand(node.id, true);
    } else if (event.key === "ArrowLeft") {
      if (expanded.has(node.id)) expand(node.id, false);
      else if (parent) next = rows.findIndex((row) => row.dataset.nodeId === parent);
    } else if (event.key === "Enter" || event.key === " ") {
      if (node.children?.length) expand(node.id, !expanded.has(node.id));
      else onNodeClick?.(node);
    } else return;
    event.preventDefault();
    rows[next]?.focus();
  }
  function renderNodes(items: TreeNode[], level: number, parent?: string): ReactNode {
    return items.map((node) => {
      const open = expanded.has(node.id);
      const branch = Boolean(node.children?.length);
      return (
        <div key={node.id} role="none">
          <Row
            role="treeitem"
            data-node-id={node.id}
            aria-level={level + 1}
            aria-expanded={branch ? open : undefined}
            aria-selected={selectedNodeId === node.id}
            tabIndex={focused === node.id || (!focused && level === 0) ? 0 : -1}
            $level={level}
            $selected={selectedNodeId === node.id}
            onMouseEnter={(event) => {
              // ponytail: native title only when the label is cut off; short names get none.
              const label = event.currentTarget.lastElementChild;
              event.currentTarget.title =
                label && label.scrollWidth > label.clientWidth ? node.label : "";
            }}
            onFocus={() => setFocused(node.id)}
            onKeyDown={(event) => keyDown(event, node, parent)}
            onClick={(event) => {
              event.currentTarget.focus();
              if (branch) expand(node.id, !open);
              else onNodeClick?.(node);
            }}
            onContextMenu={(event) => {
              event.preventDefault();
              event.currentTarget.focus();
              onNodeContextMenu?.(node, event);
            }}
          >
            <Disclosure aria-hidden="true">
              {branch && (open ? <ChevronDown16Regular /> : <ChevronRight16Regular />)}
            </Disclosure>
            {open && node.expandedIcon ? node.expandedIcon : node.icon}
            <Label>{node.label}</Label>
          </Row>
          {open && node.children && (
            <div role="none">{renderNodes(node.children, level + 1, node.id)}</div>
          )}
        </div>
      );
    });
  }
  return (
    <Tree role="tree" aria-label="Event logs">
      {renderNodes(nodes, 0)}
    </Tree>
  );
}
