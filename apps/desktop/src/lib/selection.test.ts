import { afterEach, expect, test } from "vitest";
import { codePointOffset, selectionOffsets } from "./selection";

afterEach(() => {
  window.getSelection()!.removeAllRanges();
  document.body.replaceChildren();
});

test.each([
  ["", 0, 0],
  ["🙂 Anna", 0, 0],
  ["🙂 Anna", 2, 1],
  ["🙂 Anna", 3, 2],
  ["🙂 Anna", 7, 6],
  ["e\u0301 Anna", 1, 1],
  ["e\u0301 Anna", 2, 2],
])("converts %j at UTF-16 offset %i to code-point offset %i", (text, offset, expected) => {
  expect(codePointOffset(text, offset)).toBe(expected);
});

test.each([-1, 8, 1, 1.5, NaN, Infinity, -Infinity])(
  "rejects invalid UTF-16 offset %s without rounding or splitting emoji",
  (offset) => {
    expect(() => codePointOffset("🙂 Anna", offset)).toThrow(RangeError);
  },
);

function textContainer(text: string) {
  const container = document.createElement("div");
  container.textContent = text;
  document.body.append(container);
  return container;
}

test("selects a name after emoji without counting sibling UI controls", () => {
  const button = document.createElement("button");
  button.textContent = "Person hinzufügen";
  document.body.append(button);
  const container = textContainer("🙂 Anna");
  const selection = window.getSelection()!;
  const range = document.createRange();
  range.setStart(container.firstChild!, 3);
  range.setEnd(container.firstChild!, 7);
  selection.addRange(range);

  expect(selection.toString()).toBe("Anna");
  expect(selectionOffsets(container, selection)).toEqual({ start: 2, end: 6 });
});

test("normalizes a backward selection across nested marks", () => {
  const container = textContainer("🙂 ");
  const mark = document.createElement("mark");
  const inner = document.createElement("mark");
  mark.append("An");
  inner.append("na");
  mark.append(inner);
  container.append(mark, " Müller");
  const selection = window.getSelection()!;
  selection.collapse(container.lastChild!, 7);
  selection.extend(mark.firstChild!, 0);

  expect(selection.toString()).toBe("Anna Müller");
  expect(selectionOffsets(container, selection)).toEqual({ start: 2, end: 13 });
});

test("uses child indices for element endpoints and counts combining code points separately", () => {
  const container = textContainer("🙂 ");
  const mark = document.createElement("mark");
  mark.textContent = "e\u0301";
  container.append(mark, " Anna");
  const selection = window.getSelection()!;
  selection.setBaseAndExtent(container, 1, mark, 1);

  expect(selection.toString()).toBe("e\u0301");
  expect(selectionOffsets(container, selection)).toEqual({ start: 2, end: 4 });
});

test("accepts whole-container boundaries including whitespace", () => {
  const container = textContainer("\n🙂 Anna\n");
  const range = document.createRange();
  range.selectNodeContents(container);
  const selection = window.getSelection()!;
  selection.addRange(range);

  expect(selectionOffsets(container, selection)).toEqual({ start: 0, end: 8 });
});

test.each(["start", "end", "both"])("rejects selection with %s outside the container", (outside) => {
  const before = textContainer("Control before");
  const container = textContainer("Anna");
  const after = textContainer("Control after");
  const selection = window.getSelection()!;
  selection.setBaseAndExtent(
    outside === "end" ? container.firstChild! : before.firstChild!, 0,
    outside === "start" ? container.firstChild! : after.firstChild!, 2,
  );

  expect(selectionOffsets(container, selection)).toBeNull();
});

test("rejects missing ranges, caret selections, and ranges with no text", () => {
  const container = textContainer("Anna");
  const selection = window.getSelection()!;
  expect(selectionOffsets(container, selection)).toBeNull();
  selection.collapse(container.firstChild!, 2);
  expect(selectionOffsets(container, selection)).toBeNull();
  const emptyMark = document.createElement("mark");
  container.append(emptyMark);
  selection.setBaseAndExtent(container, 1, container, 2);
  expect(selectionOffsets(container, selection)).toBeNull();
});

test.each([[1, 3], [0, 1]])("rejects a selection splitting an emoji at %i..%i", (start, end) => {
  const container = textContainer("🙂 Anna");
  const selection = window.getSelection()!;
  selection.setBaseAndExtent(container.firstChild!, start, container.firstChild!, end);

  expect(selectionOffsets(container, selection)).toBeNull();
});

test("rejects an element boundary between the two halves of an emoji", () => {
  const container = textContainer("\ud83d");
  const mark = document.createElement("mark");
  mark.textContent = "\ude42 Anna";
  container.append(mark);
  const selection = window.getSelection()!;
  selection.setBaseAndExtent(container, 1, mark.firstChild!, 6);

  expect(selectionOffsets(container, selection)).toBeNull();
});
