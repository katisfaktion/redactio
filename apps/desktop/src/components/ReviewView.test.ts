import { flushPromises, mount } from "@vue/test-utils";
import { effectScope } from "vue";
import { expect, test, vi } from "vitest";
import ReviewView from "./ReviewView.vue";
import { useReview } from "../composables/useReview";
import type { ReviewViewData } from "../lib/contracts";

const view: ReviewViewData = {
  key: { sync_pair_id: "11111111-1111-4111-8111-111111111111", doc_id: "doc-0001" },
  source_hash: "a".repeat(64), revision: "22222222-2222-4222-8222-222222222222", expected_output_hash: "b".repeat(64), expected_review_hash: "d".repeat(64),
  original_text: '<img src="https://example.invalid/pixel"> <PERSON_1>', body: '<img src="https://example.invalid/pixel"> <PERSON_1>', markdown: "synthetic markdown",
  detections: [], redactions: [], decisions: { dismissed_ids: [], manual: [] }, warnings: [], acknowledged_warnings: [], notes: "", status: "pending",
};
async function setup(value = view, entityTypes: string[] = []) {
  const scope = effectScope();
  const review = scope.run(() => useReview({ open: async () => value, save: async (key, input) => ({ ...value, ...input, key }) }))!;
  await review.open(value.key);
  const wrapper = mount(ReviewView, { props: { review, pairName: "Sammlung A", entityTypes }, attachTo: document.body });
  return { review, wrapper, cleanup() { wrapper.unmount(); scope.stop(); } };
}

test("review keeps an unknown native code selectable and displays it beside its fallback label", async () => {
  const native = { ...view, original_text: "Kontonummer", body: "<BILLING_ACCOUNT_1>\n", detections: [
    { id: "account", start: 0, end: 11, entity_type: "BILLING_ACCOUNT", confidence: .9, recognizer: "native", origin: "automatic" as const },
  ] };
  const { wrapper, cleanup } = await setup(native, ["BILLING_ACCOUNT", "PERSON"]);
  try {
    expect(wrapper.text()).toContain("BILLING_ACCOUNT (BILLING_ACCOUNT)");
    await wrapper.get('[data-testid="review-output"] mark').trigger("click");
    expect(wrapper.get('[role="option"][aria-label="BILLING_ACCOUNT"]').exists()).toBe(true);
  } finally { cleanup(); }
});

test("untrusted text stays inert and literal placeholders are not detections", async () => {
  const { wrapper, cleanup } = await setup();
  expect(wrapper.find("img").exists()).toBe(false);
  expect(wrapper.find("mark").exists()).toBe(false);
  expect(wrapper.get('[data-testid="review-original"]').text()).toBe(view.original_text);
  expect(wrapper.get('[data-testid="review-output"]').text()).toBe(view.body);
  expect(wrapper.text()).toContain("Sammlung A · doc-0001");
  cleanup();
});

test("readonly keyboard selection converts emoji offsets and adds a manual span", async () => {
  const { review, wrapper, cleanup } = await setup({ ...view, original_text: "🙂 Anna", body: "🙂 Anna" });
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  const source = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
  expect(source.attributes("aria-readonly")).toBe("true");
  source.element.setSelectionRange(3, 7, "backward");
  await source.trigger("select");
  await wrapper.get('[data-testid="add-redaction"]').trigger("click");
  expect(review.decisions.value.manual[0]).toMatchObject({ start: 2, end: 6, entity_type: "PERSON", confidence: null });
  expect(source.element.value).toBe("🙂 <PERSON_1>\n");
  expect(wrapper.text()).toContain("Ungespeicherte Vorschau");
  await wrapper.get('[data-testid="undo-review"]').trigger("click");
  expect(review.decisions.value.manual).toEqual([]);
  cleanup();
});

test("keyboard source blocks text edits, paste and drop and restores authoritative text on input", async () => {
  const { wrapper, cleanup } = await setup();
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  const control = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
  for (const type of ["beforeinput", "paste", "drop"]) {
    const event = new Event(type, { bubbles: true, cancelable: true });
    expect(control.element.dispatchEvent(event)).toBe(false);
  }
  control.element.value = "unwanted edit";
  control.element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertCompositionText", isComposing: true }));
  expect(control.element.value).toBe(view.original_text + "\n");
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  const reopened = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
  reopened.element.setSelectionRange(1, 4); await reopened.trigger("select");
  expect(wrapper.text()).toContain("Auswahl: Position 1–4");
  cleanup();
});

test.each([
  ["before\r\n🙂 Anna", "before\n🙂 Anna", 10, 14, 10, 14, "Anna"],
  ["x\r\nA\r\n🙂B\rz", "x\nA\n🙂B\nz", 2, 7, 3, 8, "A\r\n🙂B"],
  ["x\r\n🙂", "x\n🙂", 1, 2, 1, 3, "\r\n"],
  ["x\r🙂 Anna", "x\n🙂 Anna", 5, 9, 4, 8, "Anna"],
  ["x\r🙂\rAnna", "x\n🙂\nAnna", 2, 9, 2, 8, "🙂\rAnna"],
  ["\r\n\r\n🙂 Anna", "\n\n🙂 Anna", 5, 9, 6, 10, "Anna"],
  ["x\n🙂 Anna", "x\n🙂 Anna", 5, 9, 4, 8, "Anna"],
] as const)("native newline normalization preserves source coordinates: %j", async (original_text, nativeText, start, end, sourceStart, sourceEnd, selectedText) => {
  const { review, wrapper, cleanup } = await setup({ ...view, original_text, body: original_text });
  try {
    await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
    const source = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
    expect(source.element.value).toBe(nativeText + "\n");
    source.element.setSelectionRange(start, end, "backward");
    await source.trigger("select");
    await wrapper.get('[data-testid="add-redaction"]').trigger("click");
    const manual = review.decisions.value.manual[0]!;
    expect(manual).toMatchObject({ start: sourceStart, end: sourceEnd });
    expect(Array.from(original_text).slice(manual.start, manual.end).join("")).toBe(selectedText);
    expect(review.data.value!.original_text).toBe(original_text);
  } finally { cleanup(); }
});

test("normalized native offsets still reject partial surrogate pairs after CRLF", async () => {
  const { wrapper, cleanup } = await setup({ ...view, original_text: "x\r\n🙂Z" });
  try {
    await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
    const source = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
    expect(source.element.value).toBe("x\n🙂Z\n");
    for (const [start, end] of [[2, 3], [3, 4]]) {
      source.element.setSelectionRange(start!, end!); await source.trigger("select");
      expect(wrapper.get('[data-testid="add-redaction"]').attributes("disabled")).toBeDefined();
    }
    source.element.setSelectionRange(2, 4); await source.trigger("select");
    expect(wrapper.text()).toContain("Auswahl: Position 3–4");
  } finally { cleanup(); }
});

test("known overlapping spans highlight their union without changing selectable text", async () => {
  const { wrapper, cleanup } = await setup({ ...view, original_text: "🙂 Anna Müller", body: "🙂 Anna Müller", detections: [
    { id: "a", start: 2, end: 6, entity_type: "PERSON", confidence: .8, recognizer: "test", origin: "automatic" },
    { id: "b", start: 4, end: 13, entity_type: "PERSON", confidence: .9, recognizer: "test", origin: "automatic" },
  ] });
  expect(wrapper.get('[data-testid="review-original"]').element.textContent).toBe("🙂 Anna Müller");
  expect(wrapper.get('[data-testid="review-original"] mark').text()).toBe("Anna Müller");
  cleanup();
});

const linkedView: ReviewViewData = {
  ...view, original_text: "🙂 Anna und Jörg.", body: "🙂 <PERSON_1> und Jörg.\n",
  detections: [{ id: "anna", start: 2, end: 6, entity_type: "PERSON", confidence: .9, recognizer: "test", origin: "automatic" }],
  redactions: [{ start_offset: 2, end_offset: 12, entity_type: "PERSON", placeholder: "<PERSON_1>", confidence: .9, recognizer: "test", origin: "automatic" }],
};

test("selection in the masked preview maps to the original and updates the draft immediately", async () => {
  const { review, wrapper, cleanup } = await setup(linkedView);
  try {
    const output = wrapper.get('[data-testid="review-output"]');
    const walker = document.createTreeWalker(output.element, NodeFilter.SHOW_TEXT);
    let tail = walker.nextNode()!;
    while (!tail.textContent!.includes("Jörg")) tail = walker.nextNode()!;
    const offset = tail.textContent!.indexOf("Jörg");
    const selection = window.getSelection()!;
    selection.setBaseAndExtent(tail, offset, tail, offset + 4);
    await output.trigger("mouseup");
    await wrapper.get('[data-testid="add-redaction"]').trigger("click");
    expect(review.decisions.value.manual[0]).toMatchObject({ start: 11, end: 15 });
    expect(wrapper.get('[data-testid="review-output"]').text()).toBe("🙂 <PERSON_1> und <PERSON_2>.");
    expect(wrapper.get('[data-testid="review-original"]').element.textContent).toBe(linkedView.original_text);
    expect(wrapper.text()).toContain("Ungespeicherte Vorschau");
    await wrapper.get('[data-testid="undo-review"]').trigger("click");
    expect(wrapper.get('[data-testid="review-output"]').text()).toBe(linkedView.body.trim());
  } finally { window.getSelection()!.removeAllRanges(); cleanup(); }
});

test("clicking a mark updates the inspector without moving focus or scrolling the page", async () => {
  const scroll = vi.fn();
  const previous = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = scroll;
  const { wrapper, cleanup } = await setup(linkedView);
  try {
    const mark = wrapper.get<HTMLElement>('[data-testid="review-output"] mark');
    mark.element.focus();
    await mark.trigger("click");
    await flushPromises();
    expect(wrapper.get('[data-testid="redaction-inspector"]').text()).toContain("Anna");
    expect(wrapper.get('[data-testid="review-original"] mark').classes()).toContain("focused-redaction");
    expect(document.activeElement).toBe(mark.element);
    expect(scroll).not.toHaveBeenCalled();
  } finally { HTMLElement.prototype.scrollIntoView = previous; cleanup(); }
});

test("dragging over a mark keeps selection and does not activate its detail controls", async () => {
  const { wrapper, cleanup } = await setup(linkedView);
  try {
    const mark = wrapper.get('[data-testid="review-output"] mark');
    const text = mark.element.firstChild!;
    window.getSelection()!.setBaseAndExtent(text, 1, text, 4);
    await mark.trigger("mouseup");
    await mark.trigger("click");
    expect(wrapper.get('[data-testid="redaction-inspector"]').text()).toContain("Auswahl: Position 2–6");
    expect(wrapper.get('[data-testid="add-redaction"]').text()).toContain("1 Schwärzung ersetzen");
    expect(wrapper.find('[data-testid="edit-range"]').exists()).toBe(false);
    // Keyboard activation explicitly selects the detection even with a stale text selection.
    await mark.trigger("keydown", { key: "Enter" });
    expect(wrapper.get('[data-testid="edit-range"]').exists()).toBe(true);
  } finally { window.getSelection()!.removeAllRanges(); cleanup(); }
});

test("replacing a selection across two redactions removes both old spans including their overshoot", async () => {
  const value: ReviewViewData = { ...view, original_text: "Hans Gehring, wohnt in München", detections: [
    { id: "first", start: 0, end: 4, entity_type: "FIRSTNAME", confidence: .9, recognizer: "test", origin: "automatic" },
    { id: "wide", start: 7, end: 19, entity_type: "LASTNAME", confidence: .9, recognizer: "test", origin: "automatic" },
  ] };
  const { review, wrapper, cleanup } = await setup(value);
  try {
    const original = wrapper.get('[data-testid="review-original"]');
    const marks = original.findAll('mark');
    window.getSelection()!.setBaseAndExtent(marks[0]!.element.firstChild!, 0, marks[1]!.element.firstChild!, 5);
    await original.trigger('mouseup');
    const panel = wrapper.get('[data-testid="redaction-inspector"]');
    expect(panel.text()).toContain('Hans Gehring');
    expect(panel.text()).toContain(', wohnt');
    expect(wrapper.get('[data-testid="add-redaction"]').text()).toContain('2 Schwärzungen ersetzen');
    await wrapper.get('[data-testid="add-redaction"]').trigger('click');
    expect(review.active.value).toHaveLength(1);
    expect(review.decisions.value.dismissed_ids).toEqual(['first', 'wide']);
    expect(wrapper.get('[data-testid="review-output"]').text()).toBe('<PERSON_1>, wohnt in München');
    await wrapper.get('[data-testid="undo-review"]').trigger('click');
    expect(review.active.value.map(d => d.id)).toEqual(['first', 'wide']);
  } finally { window.getSelection()!.removeAllRanges(); cleanup(); }
});

test("range editing exposes exact original characters and replaces a too-wide detection", async () => {
  const { review, wrapper, cleanup } = await setup({ ...linkedView, detections: [
    { ...linkedView.detections[0]!, end: 10 },
  ] });
  try {
    await wrapper.get('[data-testid="review-output"] mark').trigger('click');
    await wrapper.get('[data-testid="edit-range"]').trigger('click');
    const input = wrapper.get<HTMLTextAreaElement>('[data-testid="range-text"]');
    expect(input.element.value).toBe(linkedView.original_text);
    expect(input.element.value.slice(input.element.selectionStart, input.element.selectionEnd)).toBe('Anna und');
    input.element.setSelectionRange(3, 7); await input.trigger('select');
    await wrapper.get('[data-testid="add-redaction"]').trigger('click');
    expect(review.active.value).toHaveLength(1);
    expect(review.active.value[0]).toMatchObject({ start: 2, end: 6 });
    expect(review.decisions.value.dismissed_ids).toEqual(['anna']);
    expect(wrapper.get('[data-testid="review-output"]').text()).toBe(linkedView.body.trim());
  } finally { cleanup(); }
});

test("keyboard selection uses the masked preview while original redactions remain visible", async () => {
  const { review, wrapper, cleanup } = await setup(linkedView);
  try {
    await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
    expect(wrapper.get('[data-testid="review-original"] mark').text()).toBe("Anna");
    const input = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
    expect(input.element.value).toBe(linkedView.body);
    const offset = input.element.value.indexOf("Jörg");
    input.element.setSelectionRange(offset, offset + 4);
    await input.trigger("select");
    await wrapper.get('[data-testid="add-redaction"]').trigger("click");
    expect(review.decisions.value.manual[0]).toMatchObject({ start: 11, end: 15 });
    expect(input.element.value).toContain("<PERSON_2>");
  } finally { cleanup(); }
});

test("warnings and empty extraction prevent approval and save failures are announced", async () => {
  const { review, wrapper, cleanup } = await setup({ ...view, original_text: "", body: "", status: "needs-rework", warnings: ["empty_document"] });
  expect(wrapper.get('[data-testid="approve-review"]').attributes("disabled")).toBeDefined();
  expect(wrapper.text()).toContain("Keine extrahierbaren Inhalte");
  review.error.value = { code: "review_conflict", retryable: false };
  await flushPromises();
  expect(wrapper.get('[role="alert"]').text()).toContain("geändert");
  expect(wrapper.find('[aria-live="polite"]').exists()).toBe(true);
  cleanup();
});

test("review mismatch directs the user back to reprocess the document", async () => {
  const { review, wrapper, cleanup } = await setup();
  review.error.value = { code: "review_mismatch", retryable: false };
  await flushPromises();
  expect(wrapper.get('[role="alert"]').text()).toContain("gespeicherte Prüfung stimmt nicht mit der aktuellen Ausgabe überein");
  expect(wrapper.get('[role="alert"]').text()).toContain("Dokumentliste zurück und verarbeiten Sie das Dokument erneut");
  cleanup();
});

test("warning acknowledgement explicitly gates approval through a labeled control", async () => {
  const { review, wrapper, cleanup } = await setup({ ...view, warnings: ["headers_footers"], status: "needs-rework" });
  expect(wrapper.get('[data-testid="approve-review"]').attributes("disabled")).toBeDefined();
  await wrapper.get('input[type="checkbox"]').setValue(true);
  expect(review.acknowledged.value).toEqual(["headers_footers"]);
  expect(wrapper.get('[data-testid="approve-review"]').attributes("disabled")).toBeUndefined();
  review.busy.value = true; await flushPromises();
  expect(wrapper.get('input[type="checkbox"]').attributes("disabled")).toBeDefined();
  cleanup();
});

test("the million-code-point limit keeps original highlights and maps a native preview selection", async () => {
  const original_text = "a ".repeat(499_997) + "🙂 Anna";
  const started = performance.now();
  const { wrapper, cleanup } = await setup({ ...view, original_text, body: original_text, detections: [
    { id: "end", start: 999_996, end: 1_000_000, entity_type: "PERSON", confidence: .8, recognizer: "test", origin: "automatic" },
  ] });
  expect(wrapper.get('[data-testid="review-original"]').element.textContent).toBe(original_text);
  expect(wrapper.get('[data-testid="review-original"] mark').text()).toBe("Anna");
  const rendered = performance.now();
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  expect(wrapper.get('[data-testid="review-original"] mark').text()).toBe("Anna");
  const source = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
  expect(source.element.value).toBe("a ".repeat(499_997) + "🙂 <PERSON_1>\n");
  source.element.setSelectionRange(999_997, 1_000_001);
  await source.trigger("select");
  expect(wrapper.text()).toContain("Auswahl: Position 999996–1000000");
  console.info(`Synthetic 1M code points: mount ${(rendered - started).toFixed(1)} ms; mode switch + selection ${(performance.now() - rendered).toFixed(1)} ms (jsdom; native timing measured separately)`);
  cleanup();
});

test("exiting original range edit preserves original and preview positions", async () => {
 const {wrapper,cleanup}=await setup({...view,original_text:"Anna und Jörg",detections:[{id:"a",start:0,end:4,entity_type:"PERSON",confidence:.9,recognizer:"test",origin:"automatic"}]});
 try {
 const original=wrapper.get<HTMLElement>('[data-testid="review-original"]');
 const output=wrapper.get<HTMLElement>('[data-testid="review-output"]');
 original.element.scrollTop=640; output.element.scrollTop=320;
 await output.get('mark').trigger('click');
 await wrapper.get('[data-testid="edit-range"]').trigger('click');
 expect(wrapper.get<HTMLTextAreaElement>('[data-testid="range-text"]').element.scrollTop).toBe(640);
 await wrapper.get('[data-testid="keyboard-select"]').trigger('click');
 expect(wrapper.get<HTMLElement>('[data-testid="review-original"]').element.scrollTop).toBe(640);
 expect(wrapper.get<HTMLElement>('[data-testid="review-output"]').element.scrollTop).toBe(320);
 } finally {cleanup();}
});
test("keyboard activation survives the keyup event after a previous selection", async () => {
 const {wrapper,cleanup}=await setup({...view,original_text:"Anna und Jörg",detections:[{id:"a",start:0,end:4,entity_type:"PERSON",confidence:.9,recognizer:"test",origin:"automatic"}]});
 try {
 const mark=wrapper.get('[data-testid="review-output"] mark');
 const text=mark.element.firstChild!;
 window.getSelection()!.setBaseAndExtent(text,1,text,4);
 await mark.trigger('mouseup');
 await mark.trigger('keydown',{key:'Enter'});
 expect(wrapper.find('[data-testid="edit-range"]').exists()).toBe(true);
 await mark.trigger('keyup',{key:'Enter'});
 expect(wrapper.find('[data-testid="edit-range"]').exists()).toBe(true);
 } finally {window.getSelection()!.removeAllRanges();cleanup();}
});
