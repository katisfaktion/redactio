import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.vue";
import PairManager from "./components/PairManager.vue";
import DocumentList from "./components/DocumentList.vue";
import ExportDialog from "./components/ExportDialog.vue";
import { detectionApi, exportApi, pairApi, runApi } from "./lib/ipc";
import type { ExportSummary, Settings } from "./lib/contracts";
import { open } from "@tauri-apps/plugin-dialog";

const native = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ onCloseRequested: native.listen }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), open: vi.fn(), save: vi.fn() }));
afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });
const pairA = "11111111-1111-4111-8111-111111111111", pairB = "22222222-2222-4222-8222-222222222222";
const settings: Settings = { schema_version: 1, selected_sync_pair_id: pairA, sync_pairs: [pairA, pairB].map((id, i) => ({
  id, name: `Sammlung ${i ? "B" : "A"}`, source_folder: `/source${i}`, target_folder: `/target${i}`, created_at: "2026-09-19T10:00:00Z", processing_revision: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  config: { model: "de_core_news_lg", enabled_entities: [], custom_rules: [], include_positions: true },
})) };
async function setup() {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value() { this.open = true; } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value() { this.open = false; } });
  vi.spyOn(detectionApi, "listModels").mockResolvedValue([]);
  vi.spyOn(detectionApi, "refresh").mockImplementation(async id => ({ ...settings, selected_sync_pair_id: id }));
  vi.spyOn(runApi, "listen").mockResolvedValue(() => {});
  const select = vi.spyOn(pairApi, "selectPair").mockImplementation(async id => ({ ...settings, selected_sync_pair_id: id }));
  vi.spyOn(pairApi, "scanPair").mockResolvedValue({ files: [{ doc_id: "doc-0001", relative_path: "example.docx", size_bytes: 5, mtime: null, source_hash_sha256: "a".repeat(64), review_status: "pending", state: "current" }], errors: [] });
  const approved = vi.spyOn(exportApi, "approved").mockImplementation(async id => ({ sync_pair_id: id, exported: ["doc-0001"], failed: [], error: null, cancelled: false, audit_warning: false }));
  let close!: (event: { preventDefault(): void }) => void;
  native.listen.mockImplementation(async callback => { close = callback; return () => {}; });
  const wrapper = mount(App, { props: { initialSettings: settings }, attachTo: document.body });
  await flushPromises();
  const scan = async () => {
    await wrapper.findAll("button").find(b => b.text() === "Quellordner einlesen")!.trigger("click"); await flushPromises();
    await wrapper.get('input[type="checkbox"]').setValue(true);
    await wrapper.get('[data-testid="export-selection"]').trigger("click"); await flushPromises();
  };
  return { wrapper, scan, approved, select, close: (event: { preventDefault(): void }) => close(event) };
}

test("live selection freezes pair identity through picker, result, and a second pair's repeated ID", async () => {
  const { wrapper, scan, approved, select, close } = await setup();
  let choose!: (path: string) => void;
  vi.mocked(open).mockImplementationOnce(() => new Promise(resolve => { choose = resolve; }));
  await scan();
  expect(wrapper.getComponent(ExportDialog).props("keys")).toEqual([{ sync_pair_id: pairA, doc_id: "doc-0001" }]);
  await wrapper.get('[data-testid="export-start"]').trigger("click"); await flushPromises();
  expect(wrapper.getComponent(PairManager).props("busy")).toBe(true);
  wrapper.getComponent(PairManager).vm.$emit("select", pairB);
  await wrapper.get('[data-testid="settings-nav"]').trigger("click");
  const preventDefault = vi.fn(); close({ preventDefault });
  expect(preventDefault).toHaveBeenCalledOnce(); expect(select).not.toHaveBeenCalled();
  choose("/export-A"); await flushPromises();
  expect(approved).toHaveBeenLastCalledWith(pairA, [{ sync_pair_id: pairA, doc_id: "doc-0001" }], "/export-A");
  expect(wrapper.get('[data-testid="exported"]').text()).toContain("doc-0001");
  expect(wrapper.getComponent(PairManager).props("busy")).toBe(true);
  await wrapper.get('[data-testid="export-close"]').trigger("click"); await flushPromises();
  expect(wrapper.getComponent(PairManager).props("busy")).toBe(false);
  wrapper.getComponent(PairManager).vm.$emit("select", pairB); await flushPromises();
  expect(wrapper.find('input[type="checkbox"]').exists()).toBe(false);
  vi.mocked(open).mockResolvedValueOnce("/export-B");
  await scan(); await wrapper.get('[data-testid="export-start"]').trigger("click"); await flushPromises();
  expect(approved).toHaveBeenLastCalledWith(pairB, [{ sync_pair_id: pairB, doc_id: "doc-0001" }], "/export-B");
  expect(wrapper.getComponent(ExportDialog).text()).toContain("Sammlung B");
  wrapper.unmount();
});

test("native picker cancellation remains an audited action and keeps its warning visible", async () => {
  const { wrapper, scan, approved } = await setup();
  vi.mocked(open).mockResolvedValue(null);
  approved.mockResolvedValue({ sync_pair_id: pairA, exported: [], failed: [], error: null, cancelled: true, audit_warning: true } satisfies ExportSummary);
  await scan(); await wrapper.get('[data-testid="export-start"]').trigger("click"); await flushPromises();
  expect(approved).toHaveBeenCalledWith(pairA, [{ sync_pair_id: pairA, doc_id: "doc-0001" }], null);
  expect(wrapper.getComponent(ExportDialog).text()).toContain("Export abgebrochen.");
  expect(wrapper.getComponent(ExportDialog).text()).toContain("Protokoll konnte nicht vollständig");
  await wrapper.get('[data-testid="export-close"]').trigger("click"); await flushPromises();
  expect(wrapper.findComponent(ExportDialog).exists()).toBe(false);
  expect(wrapper.getComponent(DocumentList).props("disabled")).toBe(false);
  wrapper.unmount();
});
