import { styled } from "styled-components";

// Header stays put; only the facet list below it scrolls, so nothing slides under it.
export const SidebarContainer = styled.div`
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  min-width: 0;
  background: ${({ theme }) => theme.colors.surface.pane};
`;

export const SidebarBody = styled.div`
  flex: 1;
  min-height: 0;
  overflow-y: auto;
`;

export const Notice = styled.p`
  padding: ${({ theme }) => theme.spacing.md};
  color: ${({ theme }) => theme.colors.text.secondary};
`;

export const Section = styled.section`
  border-bottom: 1px solid ${({ theme }) => theme.colors.stroke.divider};
`;

export const SectionHeader = styled.div`
  display: flex;
  align-items: center;
  height: ${({ theme }) => theme.size.header};
  padding-right: ${({ theme }) => theme.spacing.xs};
  &:hover {
    background: ${({ theme }) => theme.colors.fill.hover};
  }
  /* Per-section Clear shows on hover or keyboard focus only. */
  > button:last-child:not(:first-child) {
    visibility: hidden;
  }
  &:hover > button:last-child,
  &:focus-within > button:last-child {
    visibility: visible;
  }
`;

export const SectionToggle = styled.button`
  display: flex;
  flex: 1;
  align-items: center;
  gap: 6px;
  min-width: 0;
  height: 100%;
  padding: 0 ${({ theme }) => theme.spacing.sm};
  border: 0;
  background: none;
  font-weight: 600;
  text-align: left;
  cursor: default;
  svg {
    flex-shrink: 0;
    color: ${({ theme }) => theme.colors.text.secondary};
  }
  > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  &:focus-visible {
    outline-offset: -2px;
  }
`;

export const SelectedCount = styled.span`
  flex-shrink: 0;
  font-weight: 400;
  font-size: ${({ theme }) => theme.fontSize.secondary};
  color: ${({ theme }) => theme.colors.text.secondary};
  font-variant-numeric: tabular-nums;
`;

export const FacetSearch = styled.div`
  padding: 2px ${({ theme }) => theme.spacing.sm} ${({ theme }) => theme.spacing.xs};
`;

export const OptionsContainer = styled.div`
  max-height: 242px; /* 11 rows */
  overflow-y: auto;
  padding-bottom: ${({ theme }) => theme.spacing.xs};
`;

/** `--share` (0-100%) draws the value's share of the facet total behind the row. */
export const FacetRow = styled.label`
  display: flex;
  align-items: center;
  gap: ${({ theme }) => theme.spacing.sm};
  height: ${({ theme }) => theme.size.row};
  padding: 0 ${({ theme }) => theme.spacing.sm} 0 30px; /* checkbox under the section label */
  --bar: linear-gradient(
    90deg,
    color-mix(in srgb, ${({ theme }) => theme.colors.accent.rest} 8%, transparent) var(--share),
    transparent var(--share)
  );
  background: var(--bar);
  &:hover {
    background:
      linear-gradient(
        ${({ theme }) => theme.colors.fill.hover},
        ${({ theme }) => theme.colors.fill.hover}
      ),
      var(--bar);
  }
`;

export const Counts = styled.span`
  flex-shrink: 0;
  font-size: ${({ theme }) => theme.fontSize.secondary};
  color: ${({ theme }) => theme.colors.text.tertiary};
  font-variant-numeric: tabular-nums;
`;

export const Checkbox = styled.input.attrs({ type: "checkbox" })`
  flex-shrink: 0;
  width: 14px;
  height: 14px;
  margin: 0;
  accent-color: ${({ theme }) => theme.colors.accent.rest};
`;

export const OptionLabel = styled.span`
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;
