import styled from "styled-components";

/**
 * Simple scroll container used in sidebars and panels where the content can
 * grow vertically. It does *not* impose a max-height – consumers should wrap
 * it in a flex parent or add their own constraints.
 */
export const ScrollableList = styled.div`
  flex: 1 1 auto;
  overflow: auto;
`;
