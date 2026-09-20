import { flushPromises, mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { expect, test, vi } from "vitest";
import DocumentList from "./DocumentList.vue";
import type { ScanReport } from "../lib/contracts";

const pairA = "11111111-1111-4111-8111-111111111111";
const pairB = "22222222-2222-4222-8222-222222222222";
const report: ScanReport = {
  files: [{
    doc_id: null, relative_path: "nested/document.docx",
    size_bytes: 9,
    mtime: "2026-09-19T10:00:00Z",
    source_hash_sha256: "b3cc0475bb78a5026098858e9889acf666d31062d513d303314eca31d36e72f2",
    state: "new", review_status: null,
  }],
  errors: [],
};

function mountList(scan: (pairId: string) => Promise<ScanReport>, pairId = pairA, onBusy?: (value: boolean) => void) {
  return mount(DocumentList, {
    props: { pairId, scan, onBusy },
    global: {
      plugins: [createOnyx({
        i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } },
      })],
    },
  });
}

test("an explicit scan displays the selected pair's documents in the table", async () => {
  const scan = vi.fn(async () => report);
  const wrapper = mountList(scan);

  await wrapper.get("button").trigger("click");

  expect(scan).toHaveBeenCalledWith(pairA);
  expect(wrapper.text()).toContain("nested/document.docx");
  expect(wrapper.text()).toContain("Neu");
  expect(wrapper.text()).toContain("2026");
  expect(wrapper.emitted("busy")).toEqual([[true], [false]]);
});

test("no eligible documents is distinct from scan and per-file failures", async () => {
  const empty = mountList(async () => ({ files: [], errors: [] }));
  await empty.get("button").trigger("click");
  expect(empty.text()).toContain("Keine geeigneten DOCX-Dokumente gefunden");

  const partial = mountList(async () => ({
    files: report.files,
    errors: [{ relative_path: "blocked.docx", code: "permission_denied" }],
  }));
  await partial.get("button").trigger("click");
  expect(partial.text()).toContain("blocked.docx");
  expect(partial.text()).toContain("konnte nicht gelesen werden");

  const failed = mountList(async () => Promise.reject({ code: "path_unavailable", retryable: false }));
  await failed.get("button").trigger("click");
  await flushPromises();
  expect(failed.get('[role="alert"]').text()).toContain("Quellordner konnte nicht eingelesen werden");
  expect(failed.text()).not.toContain("Keine geeigneten DOCX-Dokumente gefunden");
  expect(failed.emitted("busy")).toEqual([[true], [false]]);
});

test("a late scan response cannot populate a newly selected pair", async () => {
  let resolveA!: (value: ScanReport) => void;
  const scan = (pairId: string) => pairId === pairA
    ? new Promise<ScanReport>((resolve) => { resolveA = resolve; })
    : Promise.resolve({ ...report, files: [{ ...report.files[0], relative_path: "pair-b.docx" }] });
  const wrapper = mountList(scan);

  await wrapper.get("button").trigger("click");
  await wrapper.setProps({ pairId: pairB });
  resolveA(report);
  await Promise.resolve();

  expect(wrapper.text()).not.toContain("nested/document.docx");
  await wrapper.get("button").trigger("click");
  expect(wrapper.text()).toContain("pair-b.docx");
});

test("reviewable count and selected reprocessing use exact stable IDs", async () => {
  const wrapper = mountList(async () => ({ ...report, files: [{ ...report.files[0], doc_id: "doc-0001", review_status: "pending", state: "current" }] }));
  await wrapper.get("button").trigger("click");
  expect(wrapper.emitted("scanned")).toHaveLength(1);
  expect(wrapper.text()).toContain("1 prüfbar");
  await wrapper.get('input[type="checkbox"]').setValue(true);
  const reprocess = wrapper.findAll("button").find((button) => button.text().includes("erneut verarbeiten"))!;
  await reprocess.trigger("click");
  expect(wrapper.emitted("reprocess")?.[0]).toEqual([[{ relative_path: "nested/document.docx", doc_id: "doc-0001" }]]);
  await wrapper.setProps({ disabled: true });
  expect(wrapper.findAll("button").every((button) => button.attributes("disabled") !== undefined)).toBe(true);
});


test("export emits a copied full-key selection and clears it on pair changes", async () => {
  const wrapper = mountList(async () => ({ ...report, files: [{ ...report.files[0], doc_id: "doc-0001", review_status: "pending", state: "current" }] }));
  await wrapper.get("button").trigger("click");
  const start = () => wrapper.get('[data-testid="export-selection"]');
  expect(start().attributes("disabled")).toBeDefined();
  await wrapper.get('input[type="checkbox"]').setValue(true);
  await start().trigger("click");
  const emitted = wrapper.emitted("export")![0]![0];
  expect(emitted).toEqual([{ sync_pair_id: pairA, doc_id: "doc-0001" }]);
  await wrapper.setProps({ pairId: pairB });
  await wrapper.get("button").trigger("click");
  expect(start().attributes("disabled")).toBeDefined();
  await wrapper.get('input[type="checkbox"]').setValue(true);
  await start().trigger("click");
  expect(wrapper.emitted("export")![1]![0]).toEqual([{ sync_pair_id: pairB, doc_id: "doc-0001" }]);
  expect(emitted).toEqual([{ sync_pair_id: pairA, doc_id: "doc-0001" }]);
  wrapper.unmount();
});


test("review labels distinguish validated states and invalidate after a pair-owned change", async () => {
  const statuses = ["pending", "approved", "rejected", "needs-rework"] as const;
  const refreshed = { files: [{
    ...report.files[0]!, relative_path: "0.docx", doc_id: "doc-0001", state: "current" as const, review_status: "approved" as const,
  }], errors: [] };
  const scan = vi.fn()
    .mockResolvedValueOnce({ files: statuses.map((review_status, index) => ({
      ...report.files[0]!, relative_path: `${index}.docx`, doc_id: `doc-000${index + 1}`, state: "current", review_status,
    })), errors: [] })
    .mockResolvedValueOnce(refreshed);
  const wrapper = mountList(scan);
  await wrapper.get("button").trigger("click");
  for (const label of ["Ausstehend", "Freigegeben", "Abgelehnt", "Nacharbeit erforderlich"]) expect(wrapper.text()).toContain(label);
  await wrapper.setProps({ invalidation: { sync_pair_id: pairB, doc_id: "doc-0001" } });
  expect(wrapper.text()).toContain("Freigegeben");
  await wrapper.setProps({ invalidation: { sync_pair_id: pairA, doc_id: "missing" } });
  expect(scan).toHaveBeenCalledTimes(1);
  await wrapper.setProps({ invalidation: { sync_pair_id: pairA, doc_id: "doc-0001" } });
  await flushPromises();
  expect(scan).toHaveBeenCalledTimes(2);
  expect(wrapper.text()).toContain("Freigegeben");
  expect(wrapper.text()).not.toContain("Nacharbeit erforderlich");
  expect(wrapper.emitted("invalidated")).toHaveLength(1);
});

test("a matching invalidation waits until global work finishes before refreshing", async () => {
  const initial = { files: [{ ...report.files[0]!, doc_id: "doc-0001", state: "current" as const, review_status: "pending" as const }], errors: [] };
  const refreshed = { files: [{ ...initial.files[0]!, review_status: "approved" as const }], errors: [] };
  const scan = vi.fn().mockResolvedValueOnce(initial).mockResolvedValueOnce(refreshed);
  const wrapper = mountList(scan);
  await wrapper.get("button").trigger("click");

  await wrapper.setProps({ disabled: true, invalidation: { sync_pair_id: pairA } });
  expect(scan).toHaveBeenCalledTimes(1);
  expect(wrapper.text()).not.toContain("Ausstehend");
  expect(wrapper.emitted("invalidated")).toHaveLength(1);

  await wrapper.setProps({ disabled: false });
  await flushPromises();
  expect(scan).toHaveBeenCalledTimes(2);
  expect(wrapper.text()).toContain("Freigegeben");
});

test("changing pairs cancels a deferred invalidation refresh", async () => {
  const scan = vi.fn(async () => ({ files: [{ ...report.files[0]!, doc_id: "doc-0001", state: "current" as const }], errors: [] }));
  const wrapper = mountList(scan);
  await wrapper.get("button").trigger("click");
  await wrapper.setProps({ disabled: true, invalidation: { sync_pair_id: pairA } });

  await wrapper.setProps({ pairId: pairB, disabled: false });
  await flushPromises();
  expect(scan).toHaveBeenCalledTimes(1);

  await wrapper.get("button").trigger("click");
  expect(scan).toHaveBeenLastCalledWith(pairB);
});

test("pair changes and unmounts release an active scan", async () => {
  let finishPair!: (value: ScanReport) => void;
  const pairScan = new Promise<ScanReport>(resolve => { finishPair = resolve; });
  const changed = mountList(() => pairScan);
  await changed.get("button").trigger("click");
  expect(changed.emitted("busy")).toEqual([[true]]);
  await changed.setProps({ pairId: pairB });
  expect(changed.emitted("busy")).toEqual([[true], [false]]);
  finishPair(report); await flushPromises();
  expect(changed.emitted("scanned")).toBeUndefined();

  let finishUnmount!: (value: ScanReport) => void;
  const unmountScan = new Promise<ScanReport>(resolve => { finishUnmount = resolve; });
  const onBusy = vi.fn();
  const unmounted = mountList(() => unmountScan, pairA, onBusy);
  await unmounted.get("button").trigger("click");
  unmounted.unmount();
  expect(onBusy.mock.calls).toEqual([[true], [false]]);
  finishUnmount(report); await flushPromises();
  expect(unmounted.emitted("scanned")).toBeUndefined();
});
