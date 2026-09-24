import { styled } from "styled-components";
import { Button, type ButtonProps } from "./Button";
import { Tooltip } from "./Popover";

export const Toolbar = styled.div`
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px;
  min-height: 32px;
  padding: 4px 6px;
  background: ${({ theme }) => theme.colors.surface.base};
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
`;

export const ToolbarSeparator = styled.div.attrs({
  role: "separator",
  "aria-orientation": "vertical",
})`
  width: 1px;
  height: 16px;
  margin: 0 4px;
  background: ${({ theme }) => theme.colors.stroke.control};
`;

export interface ToolbarButtonProps extends Omit<ButtonProps, "variant"> {
  /** Icon-only buttons need this; it becomes the accessible name and the tooltip. */
  label?: string;
}

export function ToolbarButton({ label, active, children, ...props }: ToolbarButtonProps) {
  const button = (
    <Button
      variant="subtle"
      active={active}
      aria-pressed={active}
      aria-label={children ? undefined : label}
      {...props}
    >
      {children}
    </Button>
  );
  return label ? <Tooltip label={label}>{button}</Tooltip> : button;
}
