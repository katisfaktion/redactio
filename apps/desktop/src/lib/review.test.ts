import { beforeEach, expect, test, vi } from "vitest";
import { ReviewViewDataSchema, SaveReviewSchema } from "./contracts";
import { reviewApi } from "./ipc";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const key = { sync_pair_id: "11111111-1111-4111-8111-111111111111", doc_id: "doc-0001" };
const view = {
  key, source_hash: "a".repeat(64), revision: "22222222-2222-4222-8222-222222222222",
  expected_output_hash: "b".repeat(64), original_text: "😀 Anna", markdown: "synthetic markdown", body: "😀 <PERSON_1>\n",
  detections: [{ id: "automatic", start: 2, end: 6, entity_type: "PERSON", confidence: 0.9, recognizer: "recognizer", origin: "automatic" }],
  redactions: [{ start_offset: 2, end_offset: 12, entity_type: "PERSON", placeholder: "<PERSON_1>", confidence: 0.9, recognizer: "recognizer", origin: "automatic" }],
  decisions: { dismissed_ids: [], manual: [] }, warnings: [], acknowledged_warnings: [], notes: "PRIVATE_NOTE", status: "pending",
};
beforeEach(() => invoke.mockReset());

test("review IPC routes the complete key and validates private responses", async () => {
  invoke.mockResolvedValue(view);
  expect(await reviewApi.open(key)).toEqual(view);
  expect(invoke).toHaveBeenLastCalledWith("open_review", { key });
  const input = SaveReviewSchema.parse({ expected_output_hash: view.expected_output_hash, decisions: view.decisions,
    status: "approved", notes: "PRIVATE_NOTE", acknowledged_warnings: [] });
  expect(await reviewApi.save(key, input)).toEqual(view);
  expect(invoke).toHaveBeenLastCalledWith("save_review", { key, expectedOutputHash: input.expected_output_hash,
    decisions: input.decisions, status: "approved", notes: "PRIVATE_NOTE", acknowledgedWarnings: [] });
  invoke.mockResolvedValue({ ...view, key: { ...key, sync_pair_id: "33333333-3333-4333-8333-333333333333" } });
  await expect(reviewApi.open(key)).rejects.toThrow();
  await expect(reviewApi.save(key, input)).rejects.toThrow();
});

test("strict review projection rejects corrupt spans, decisions and approval policy", () => {
  expect(ReviewViewDataSchema.parse(view)).toEqual(view);
  for (const changed of [
    { ...view, extra: true },
    { ...view, key: { ...key, doc_id: "doc-0000" } },
    { ...view, source_hash: "unsafe" },
    { ...view, detections: [{ ...view.detections[0], end: 7 }] },
    { ...view, detections: [{ ...view.detections[0], entity_type: "UNKNOWN" }] },
    { ...view, detections: [view.detections[0], view.detections[0]] },
    { ...view, redactions: [{ ...view.redactions[0], end_offset: 14 }] },
    { ...view, decisions: { dismissed_ids: ["missing"], manual: [] } },
    { ...view, decisions: { dismissed_ids: [], manual: [{ ...view.detections[0], origin: "manual" }] } },
    { ...view, acknowledged_warnings: ["unknown_warning"] },
    { ...view, status: "approved", warnings: ["headers_footers"] },
    { ...view, status: "approved", original_text: "", detections: [], redactions: [] },
  ]) expect(ReviewViewDataSchema.safeParse(changed).success).toBe(false);
  expect(ReviewViewDataSchema.safeParse({ ...view, status: "approved", warnings: ["headers_footers"], acknowledged_warnings: ["headers_footers"] }).success).toBe(true);
});

test("outgoing review edits reject invalid identity or unknown fields before invoke", async () => {
  const input = { expected_output_hash: view.expected_output_hash, decisions: { dismissed_ids: [], manual: [] },
    status: "pending" as const, notes: "", acknowledged_warnings: [] };
  await expect(reviewApi.open({ ...key, doc_id: "../private" })).rejects.toThrow();
  await expect(reviewApi.save(key, { ...input, expected_output_hash: "bad" })).rejects.toThrow();
  expect(SaveReviewSchema.safeParse({ ...input, source_path: "/outside" }).success).toBe(false);
  expect(invoke).not.toHaveBeenCalled();
});

test("review projection accepts host safe-code bounds and preserves a BOM text character", () => {
  expect(ReviewViewDataSchema.safeParse({ ...view, warnings: ["w".repeat(128)] }).success).toBe(true);
  expect(ReviewViewDataSchema.safeParse({ ...view, original_text: "\uFEFF", body: "\uFEFF\n", detections: [], redactions: [] }).success).toBe(true);
});
