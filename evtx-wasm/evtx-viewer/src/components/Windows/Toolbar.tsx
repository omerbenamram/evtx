import { styled } from "styled-components";
import { Button, type ButtonProps } from "./Button";

export const Toolbar = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  min-height: 40px;
  padding: 4px 8px;
  background: ${({ theme }) => theme.colors.background.tertiary};
  border-bottom: 1px solid ${({ theme }) => theme.colors.border.medium};
`;

export const ToolbarSeparator = styled.div.attrs({
  role: "separator",
  "aria-orientation": "vertical",
})`
  width: 1px;
  height: 20px;
  margin: 0 4px;
  background: ${({ theme }) => theme.colors.border.medium};
`;

export interface ToolbarButtonProps extends ButtonProps {
  isActive?: boolean;
}

export function ToolbarButton({ isActive = false, ...props }: ToolbarButtonProps) {
  return (
    <Button
      variant="subtle"
      active={isActive}
      aria-pressed={isActive || undefined}
      aria-label={props["aria-label"] ?? props.title}
      {...props}
    />
  );
}
