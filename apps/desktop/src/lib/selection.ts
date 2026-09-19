export function codePointOffset(text: string, utf16Offset: number): number {
  if (!Number.isInteger(utf16Offset) || utf16Offset < 0 || utf16Offset > text.length) {
    throw new RangeError("Invalid UTF-16 offset");
  }
  const before = text.charCodeAt(utf16Offset - 1);
  const after = text.charCodeAt(utf16Offset);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) {
    throw new RangeError("Selection splits a code point");
  }
  return Array.from(text.slice(0, utf16Offset)).length;
}

// The container must contain only the original text and marks; keep UI controls outside.
export function selectionOffsets(
  container: HTMLElement,
  selection: Selection,
): { start: number; end: number } | null {
  if (selection.rangeCount !== 1) return null;
  const range = selection.getRangeAt(0);
  if (range.collapsed || !container.contains(range.startContainer)
    || !container.contains(range.endContainer)) return null;

  const prefix = container.ownerDocument.createRange();
  prefix.selectNodeContents(container);
  prefix.setEnd(range.startContainer, range.startOffset);
  const startUtf16 = prefix.toString().length;
  prefix.setEnd(range.endContainer, range.endOffset);
  const endUtf16 = prefix.toString().length;
  if (startUtf16 === endUtf16) return null;

  // Validate against the complete text, including surrogate pairs split across nodes.
  const text = container.textContent ?? "";
  try {
    return { start: codePointOffset(text, startUtf16), end: codePointOffset(text, endUtf16) };
  } catch (error) {
    if (error instanceof RangeError) return null;
    throw error;
  }
}
