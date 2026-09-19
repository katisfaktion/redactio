import { effectScope } from "vue";
import { expect, test, vi } from "vitest";
import { useReview } from "./useReview";
import type { ReviewViewData } from "../lib/contracts";
import type { ReviewApi } from "../lib/ipc";

const view: ReviewViewData = {
  key: { sync_pair_id: "11111111-1111-4111-8111-111111111111", doc_id: "doc-0001" },
  source_hash: "a".repeat(64), revision: "22222222-2222-4222-8222-222222222222", expected_output_hash: "b".repeat(64),
  original_text: "🙂 Anna", body: "🙂 <PERSON_1>", markdown: "synthetic markdown",
  detections: [{ id: "auto", start: 2, end: 6, entity_type: "PERSON", confidence: .9, origin: "automatic", recognizer: "test" }],
  redactions: [{ start_offset: 2, end_offset: 12, entity_type: "PERSON", confidence: .9, origin: "automatic", recognizer: "test", placeholder: "<PERSON_1>" }],
  decisions: { dismissed_ids: [], manual: [] }, warnings: [], acknowledged_warnings: [], notes: "", status: "pending",
};
const api: ReviewApi = { open: async () => structuredClone(view), save: async (key, input) => ({ ...view, ...input, key, expected_output_hash: "c".repeat(64) }) };

test("edits, type replacement and undo keep decisions local until a bound save", async () => {
  const scope = effectScope(), save = vi.fn(api.save);
  const review = scope.run(() => useReview({ ...api, save }))!;
  await review.open(view.key);
  review.changeType("auto", "CUSTOM");
  expect(review.decisions.value.dismissed_ids).toEqual(["auto"]);
  expect(review.decisions.value.manual[0]).toMatchObject({ start: 2, end: 6, entity_type: "CUSTOM", origin: "manual", confidence: null });
  expect(review.dirty.value).toBe(true);
  expect(review.data.value?.body).toBe("🙂 <PERSON_1>");
  review.undo();
  expect(review.decisions.value).toEqual(view.decisions);
  expect(review.dirty.value).toBe(false);
  review.add({ start: 2, end: 6 }, "PERSON");
  review.notes.value = "private note";
  await review.save();
  expect(save).toHaveBeenCalledWith(view.key, expect.objectContaining({ expected_output_hash: view.expected_output_hash, notes: "private note", status: "pending" }));
  expect(review.data.value?.expected_output_hash).toBe("c".repeat(64));
  expect(review.dirty.value).toBe(false);
  review.dismiss(review.decisions.value.manual[0]!.id);
  expect(review.decisions.value.manual).toEqual([]);
  scope.stop();
});

test("approval is separate from changed decisions and respects empty text and warnings", async () => {
  const scope = effectScope(), save = vi.fn(api.save);
  const review = scope.run(() => useReview({ ...api, save }))!;
  await review.open(view.key);
  review.add({ start: 0, end: 1 }, "CUSTOM");
  expect(review.canApprove.value).toBe(false);
  expect(await review.save("approved")).toBe(false);
  expect(save).not.toHaveBeenCalled();
  await review.save();
  expect(review.canApprove.value).toBe(true);
  await review.save("approved");
  expect(review.status.value).toBe("approved");
  await review.save("rejected");
  expect(review.status.value).toBe("rejected");
  scope.stop();
  const warnedScope = effectScope();
  const warned = warnedScope.run(() => useReview({ ...api, open: async () => ({ ...view, warnings: ["headers_footers"], status: "needs-rework" }) }))!;
  await warned.open(view.key);
  expect(warned.canApprove.value).toBe(false);
  warned.acknowledged.value = ["headers_footers"];
  expect(warned.canApprove.value).toBe(true);
  warnedScope.stop();
  const emptyScope = effectScope();
  const empty = emptyScope.run(() => useReview({ ...api, open: async () => ({ ...view, original_text: " \n", detections: [], redactions: [], status: "needs-rework" }) }))!;
  await empty.open(view.key);
  expect(empty.canApprove.value).toBe(false);
  emptyScope.stop();
});

test("conflicts preserve unsaved notes and decisions", async () => {
  const scope = effectScope();
  const review = scope.run(() => useReview({ ...api, save: async () => { throw { code: "review_conflict", retryable: false }; } }))!;
  await review.open(view.key);
  review.notes.value = "keep me";
  expect(await review.save()).toBe(false);
  expect(review.notes.value).toBe("keep me");
  expect(review.dirty.value).toBe(true);
  expect(review.error.value?.code).toBe("review_conflict");
  scope.stop();
});

test.each(["open", "save"] as const)("late %s results never replace the same document ID in another pair", async (operation) => {
  const scope = effectScope();
  let resolve!: (value: ReviewViewData) => void;
  const pending = new Promise<ReviewViewData>(done => { resolve = done; });
  const other = { ...view, key: { ...view.key, sync_pair_id: "33333333-3333-4333-8333-333333333333" }, notes: "pair B" };
  const review = scope.run(() => useReview({
    open: async key => key.sync_pair_id === other.key.sync_pair_id ? other : operation === "open" ? pending : view,
    save: () => pending,
  }))!;
  const opening = review.open(view.key);
  if (operation === "save") await opening;
  const saving = operation === "save" ? review.save() : Promise.resolve();
  await review.open(other.key);
  resolve(view);
  await opening; await saving;
  expect(review.data.value?.key).toEqual(other.key);
  expect(review.notes.value).toBe("pair B");
  expect(review.canUndo.value).toBe(false);
  scope.stop();
});

test("unmount invalidates an outstanding request and drops private document text", async () => {
  const scope = effectScope();
  let resolve!: (value: ReviewViewData) => void;
  const review = scope.run(() => useReview({ ...api, open: () => new Promise(done => { resolve = done; }) }))!;
  const request = review.open(view.key);
  scope.stop(); resolve(view); await request;
  expect(review.data.value).toBeNull();
});
