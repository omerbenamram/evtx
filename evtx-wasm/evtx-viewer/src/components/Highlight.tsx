import type { ReactNode } from "react";
import { styled } from "styled-components";

// Inline with no padding, so marking a word never moves text.
const Mark = styled.mark`
  padding: 0;
  background: color-mix(in srgb, ${({ theme }) => theme.colors.accent.rest} 20%, transparent);
  color: inherit;
`;

/** `text` with each search word marked, ignoring case. */
export function highlight(text: string, words: string[]): ReactNode {
  if (!words.length) return text;
  const pattern = words.map((word) => word.replace(/[\\^$.|?*+()[\]{}]/g, "\\$&")).join("|");
  // split with a capture group puts the matches at odd indexes.
  const parts = text.split(new RegExp(`(${pattern})`, "i"));
  return parts.length < 2
    ? text
    : parts.map((part, index) => (index % 2 ? <Mark key={index}>{part}</Mark> : part));
}
