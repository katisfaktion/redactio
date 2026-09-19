import { flushPromises, mount } from "@vue/test-utils";
import { effectScope } from "vue";
import { expect, test } from "vitest";
import ReviewView from "./ReviewView.vue";
import { useReview } from "../composables/useReview";
import type { ReviewViewData } from "../lib/contracts";

const view: ReviewViewData = {
  key: { sync_pair_id: "11111111-1111-4111-8111-111111111111", doc_id: "doc-0001" },
  source_hash: "a".repeat(64), revision: "22222222-2222-4222-8222-222222222222", expected_output_hash: "b".repeat(64),
  original_text: '<img src="https://example.invalid/pixel"> <PERSON_1>', body: '<img src="https://example.invalid/pixel"> <PERSON_1>', markdown: "synthetic markdown",
  detections: [], redactions: [], decisions: { dismissed_ids: [], manual: [] }, warnings: [], acknowledged_warnings: [], notes: "", status: "pending",
};
async function setup(value = view) {
  const scope = effectScope();
  const review = scope.run(() => useReview({ open: async () => value, save: async (key, input) => ({ ...value, ...input, key }) }))!;
  await review.open(value.key);
  const wrapper = mount(ReviewView, { props: { review, pairName: "Sammlung A" }, attachTo: document.body });
  return { review, wrapper, cleanup() { wrapper.unmount(); scope.stop(); } };
}

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
  expect(wrapper.get('[data-testid="review-output"]').text()).toBe("🙂 Anna");
  expect(wrapper.text()).toContain("Ausgabe des letzten gespeicherten Stands");
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
  expect(control.element.value).toBe(view.original_text);
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  const reopened = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
  reopened.element.setSelectionRange(1, 4); await reopened.trigger("select");
  expect(wrapper.text()).toContain("Auswahl: Position 1–4");
  cleanup();
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

test("the million-code-point limit renders exact text and switches to a single native selection control", async () => {
  const original_text = "a ".repeat(499_997) + "🙂 Anna";
  const started = performance.now();
  const { wrapper, cleanup } = await setup({ ...view, original_text, body: original_text, detections: [
    { id: "end", start: 999_996, end: 1_000_000, entity_type: "PERSON", confidence: .8, recognizer: "test", origin: "automatic" },
  ] });
  expect(wrapper.get('[data-testid="review-original"]').element.textContent).toBe(original_text);
  expect(wrapper.get('[data-testid="review-original"] mark').text()).toBe("Anna");
  const rendered = performance.now();
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  expect(wrapper.find('[data-testid="review-original"]').exists()).toBe(false);
  const source = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
  expect(source.element.value).toBe(original_text);
  source.element.setSelectionRange(999_997, 1_000_001);
  await source.trigger("select");
  expect(wrapper.text()).toContain("Auswahl: Position 999996–1000000");
  console.info(`Synthetic 1M code points: mount ${(rendered - started).toFixed(1)} ms; mode switch + selection ${(performance.now() - rendered).toFixed(1)} ms (jsdom; native timing measured separately)`);
  cleanup();
});
