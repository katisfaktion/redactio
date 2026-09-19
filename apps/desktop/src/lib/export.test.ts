import { beforeEach, expect, test, vi } from "vitest";
import { ExportSummarySchema } from "./contracts";
import { exportApi } from "./ipc";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const pair = "11111111-1111-4111-8111-111111111111";
const other = "22222222-2222-4222-8222-222222222222";
const key = { sync_pair_id: pair, doc_id: "doc-0001" };
const summary = { sync_pair_id: pair, exported: [key.doc_id], failed: [], error: null, cancelled: false, audit_warning: false };
beforeEach(() => invoke.mockReset());

test("export routes full selected keys to one explicit pair and validates its response", async () => {
  invoke.mockResolvedValue(summary);
  expect(await exportApi.approved(pair, [key], "/export")).toEqual(summary);
  expect(invoke).toHaveBeenCalledWith("export_approved", { pairId: pair, docIds: [key.doc_id], destination: "/export" });
  invoke.mockResolvedValue({ ...summary, sync_pair_id: other });
  await expect(exportApi.approved(pair, [key], "/export")).rejects.toThrow();
  invoke.mockResolvedValue({ ...summary, exported: ["doc-0002"] });
  await expect(exportApi.approved(pair, [key], "/export")).rejects.toThrow();
});

test("mixed-pair keys, duplicates and invalid IDs are rejected before invoke", async () => {
  for (const keys of [[], [key, key], [key, { ...key, sync_pair_id: other }], [{ ...key, doc_id: "../private" }]]) {
    await expect(exportApi.approved(pair, keys, "/export")).rejects.toThrow();
  }
  expect(invoke).not.toHaveBeenCalled();
});

test("cancellation crosses the real command boundary and partial results remain explicit", async () => {
  const cancelled = { ...summary, exported: [], cancelled: true };
  invoke.mockResolvedValue(cancelled);
  expect(await exportApi.approved(pair, [key], null)).toEqual(cancelled);
  expect(invoke).toHaveBeenCalledWith("export_approved", { pairId: pair, docIds: [key.doc_id], destination: null });
  const partial = { ...summary, failed: [{ doc_id: "doc-0002", error: { code: "storage_durability_uncertain", retryable: false } }], audit_warning: true };
  expect(ExportSummarySchema.parse(partial)).toEqual(partial);
  for (const invalid of [
    { ...summary, private_path: "/private" },
    { ...summary, exported: ["doc-0001", "doc-0001"] },
    { ...summary, failed: [{ doc_id: "doc-0001", error: { code: "approval_required", retryable: false } }] },
    { ...summary, cancelled: true },
  ]) expect(ExportSummarySchema.safeParse(invalid).success).toBe(false);
});
