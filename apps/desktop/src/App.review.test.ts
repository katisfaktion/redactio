import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.vue";
import PairManager from "./components/PairManager.vue";
import DocumentList from "./components/DocumentList.vue";
import ExportDialog from "./components/ExportDialog.vue";
import ReviewView from "./components/ReviewView.vue";
import { detectionApi, pairApi, reviewApi, runApi } from "./lib/ipc";
import type { ReviewViewData, Settings } from "./lib/contracts";
import { OnyxModal } from "sit-onyx";

const native = vi.hoisted(() => ({ listen: vi.fn(), destroy: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ onCloseRequested: native.listen, destroy: native.destroy }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), open: vi.fn(), save: vi.fn() }));
afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });

const pairA = "11111111-1111-4111-8111-111111111111", pairB = "22222222-2222-4222-8222-222222222222";
const settings: Settings = { schema_version: 1, selected_sync_pair_id: pairA, sync_pairs: [pairA, pairB].map((id, index) => ({
  id, name: `Sammlung ${index ? "B" : "A"}`, source_folder: `/source${index}`, target_folder: `/target${index}`, created_at: "2026-09-19T10:00:00Z", processing_revision: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  config: { model: "de_core_news_lg", enabled_entities: [], custom_rules: [], include_positions: true },
})) };
const view: ReviewViewData = {
  key: { sync_pair_id: pairA, doc_id: "doc-0001" }, source_hash: "a".repeat(64), revision: settings.sync_pairs[0]!.processing_revision,
  expected_output_hash: "b".repeat(64), original_text: "🙂 Anna", markdown: "synthetic markdown", body: "🙂 Anna", detections: [], redactions: [],
  decisions: { dismissed_ids: [], manual: [] }, warnings: [], acknowledged_warnings: [], notes: "", status: "pending",
};
async function setup() {
  vi.spyOn(detectionApi, "listModels").mockResolvedValue([]);
  vi.spyOn(detectionApi, "refresh").mockImplementation(async id => ({ ...settings, selected_sync_pair_id: id }));
  vi.spyOn(runApi, "listen").mockResolvedValue(() => {});
  vi.spyOn(runApi, "auditLocation").mockResolvedValue("/audit");
  vi.spyOn(pairApi, "scanPair").mockResolvedValue({ files: ["doc-0001", "doc-0002"].map(doc_id => ({ doc_id, relative_path: `${doc_id}.docx`, size_bytes: 5, mtime: null, source_hash_sha256: "a".repeat(64), state: "current" as const })), errors: [] });
  vi.spyOn(reviewApi, "open").mockImplementation(async key => ({ ...view, key }));
  const save = vi.spyOn(reviewApi, "save").mockImplementation(async (key, input) => ({ ...view, ...input, key }));
  native.destroy.mockReset().mockResolvedValue(undefined);
  let close!: (event: { preventDefault(): void }) => void;
  const unlisten = vi.fn();
  native.listen.mockImplementation(async callback => { close = callback; return unlisten; });
  // jsdom has no native dialog.showModal; exercise our guard with the modal's real event contract.
  const wrapper = mount(App, { props: { initialSettings: settings }, attachTo: document.body,
    global: { stubs: { OnyxModal: { props: ["open", "label"], emits: ["update:open"], template: '<dialog v-if="open" open :aria-label="label"><slot /></dialog>' } } },
  });
  await flushPromises();
  const button = (label: string) => wrapper.findAll("button").find(item => item.text() === label)!;
  await button("Quellordner einlesen").trigger("click"); await flushPromises();
  await wrapper.get('[data-testid="review-doc-0001"]').trigger("click"); await flushPromises();
  const review = wrapper.getComponent(ReviewView).props("review");
  return { wrapper, review, save, close: (event: { preventDefault(): void }) => close(event), unlisten };
}

test.each(["stay", "discard", "save"] as const)("dirty navigation resolves %s without silently losing notes", async choice => {
  const { wrapper, review, save } = await setup();
  expect(wrapper.getComponent(PairManager).text()).toContain("doc-0001");
  review.notes.value = "keep notes";
  await wrapper.get('[data-testid="settings-nav"]').trigger("click");
  expect(wrapper.findComponent(ReviewView).exists()).toBe(true);
  await wrapper.get(`[data-testid="leave-${choice}"]`).trigger("click"); await flushPromises();
  if (choice === "stay") {
    expect(wrapper.findComponent(ReviewView).exists()).toBe(true);
    expect(review.notes.value).toBe("keep notes");
  } else expect(wrapper.get('[data-testid="settings-nav"]').attributes("aria-current")).toBe("page");
  expect(save).toHaveBeenCalledTimes(choice === "save" ? 1 : 0);
  wrapper.unmount();
});

test("a failed navigation save keeps the review and guard open", async () => {
  const { wrapper, review, save } = await setup();
  review.notes.value = "do not drop";
  save.mockRejectedValue({ code: "output_conflict", retryable: false });
  await wrapper.get('[data-testid="documents-nav"]').trigger("click");
  await wrapper.get('[data-testid="leave-save"]').trigger("click"); await flushPromises();
  expect(wrapper.findComponent(ReviewView).exists()).toBe(true);
  expect(wrapper.find('[data-testid="leave-stay"]').exists()).toBe(true);
  expect(review.notes.value).toBe("do not drop");
  wrapper.unmount();
});

test.each(["select", "add", "remove"] as const)("pair %s is held until the dirty review is discarded", async action => {
  const { wrapper, review } = await setup();
  review.notes.value = "private A";
  const select = vi.spyOn(pairApi, "selectPair").mockResolvedValue({ ...settings, selected_sync_pair_id: pairB });
  const add = vi.spyOn(pairApi, "addPair").mockResolvedValue(settings);
  const remove = vi.spyOn(pairApi, "removePair").mockResolvedValue({ ...settings, sync_pairs: [settings.sync_pairs[1]!], selected_sync_pair_id: pairB });
  const manager = wrapper.getComponent(PairManager);
  if (action === "select") manager.vm.$emit("select", pairB);
  if (action === "add") manager.vm.$emit("add", "new", "/new", "/target-new", false);
  if (action === "remove") manager.vm.$emit("remove", pairA);
  await flushPromises();
  expect(select).not.toHaveBeenCalled(); expect(add).not.toHaveBeenCalled(); expect(remove).not.toHaveBeenCalled();
  await wrapper.get('[data-testid="leave-discard"]').trigger("click"); await flushPromises();
  expect({ select, add, remove }[action]).toHaveBeenCalledTimes(1);
  expect(wrapper.findComponent(ReviewView).exists()).toBe(false);
  wrapper.unmount();
});

test("document changes use the same save/discard/stay guard", async () => {
  const { wrapper, review } = await setup();
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  review.notes.value = "private first document";
  const picker = wrapper.getComponent('[data-testid="review-document-select"]');
  picker.vm.$emit("update:modelValue", "doc-0002"); await flushPromises();
  expect(review.key.value?.doc_id).toBe("doc-0001");
  await wrapper.get('[data-testid="leave-stay"]').trigger("click");
  picker.vm.$emit("update:modelValue", "doc-0002"); await flushPromises();
  await wrapper.get('[data-testid="leave-save"]').trigger("click"); await flushPromises();
  expect(review.key.value?.doc_id).toBe("doc-0002");
  expect(review.notes.value).toBe("");
  expect(wrapper.find('[data-testid="selection-text"]').exists()).toBe(false);
  await wrapper.get('[data-testid="keyboard-select"]').trigger("click");
  const source = wrapper.get<HTMLTextAreaElement>('[data-testid="selection-text"]');
  expect(source.element.value).toBe("🙂 Anna");
  source.element.setSelectionRange(0, 2); await source.trigger("select");
  await wrapper.get('[data-testid="add-redaction"]').trigger("click");
  expect(review.decisions.value.manual[0]).toMatchObject({ start: 0, end: 1 });
  wrapper.unmount();
});

test("native close can stay, save then close, and removes its listener on unmount", async () => {
  const { wrapper, review, save, close, unlisten } = await setup();
  review.notes.value = "private note";
  const preventDefault = vi.fn();
  close({ preventDefault }); await flushPromises();
  expect(preventDefault).toHaveBeenCalledOnce();
  await wrapper.get('[data-testid="leave-stay"]').trigger("click");
  expect(native.destroy).not.toHaveBeenCalled();
  close({ preventDefault }); await flushPromises();
  await wrapper.get('[data-testid="leave-save"]').trigger("click"); await flushPromises();
  expect(save).toHaveBeenCalledOnce(); expect(native.destroy).toHaveBeenCalledOnce();
  wrapper.unmount(); expect(unlisten).toHaveBeenCalledOnce();
});

test("native listener registration completing after unmount is cleaned up", async () => {
  let finish!: (unlisten: () => void) => void;
  native.listen.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const wrapper = mount(App, { props: { initialSettings: { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null } } });
  wrapper.unmount(); const unlisten = vi.fn(); finish(unlisten); await flushPromises();
  expect(unlisten).toHaveBeenCalledOnce();
});

test("native close remains cancelled on save failure and during an active operation", async () => {
  const { wrapper, review, save, close } = await setup();
  review.notes.value = "keep";
  save.mockRejectedValue({ code: "output_conflict", retryable: false });
  const preventDefault = vi.fn();
  close({ preventDefault }); await flushPromises();
  await wrapper.get('[data-testid="leave-save"]').trigger("click"); await flushPromises();
  expect(native.destroy).not.toHaveBeenCalled();
  expect(review.dirty.value).toBe(true);
  await wrapper.get('[data-testid="leave-stay"]').trigger("click");
  review.busy.value = true; close({ preventDefault });
  expect(preventDefault).toHaveBeenCalledTimes(2);
  expect(native.destroy).not.toHaveBeenCalled();
  wrapper.unmount();
});

test("dismissing the modal acts as stay and permits a later navigation request", async () => {
  const { wrapper, review } = await setup();
  review.notes.value = "keep";
  await wrapper.get('[data-testid="settings-nav"]').trigger("click");
  wrapper.getComponent(OnyxModal).vm.$emit("update:open", false); await flushPromises();
  expect(wrapper.find('[data-testid="leave-stay"]').exists()).toBe(false);
  expect(review.notes.value).toBe("keep");
  await wrapper.get('[data-testid="documents-nav"]').trigger("click");
  expect(wrapper.find('[data-testid="leave-stay"]').exists()).toBe(true);
  wrapper.unmount();
});


test("export uses the dirty-review guard and captures keys before deferred navigation", async () => {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", { configurable: true, value() { this.open = true; } });
  Object.defineProperty(HTMLDialogElement.prototype, "close", { configurable: true, value() { this.open = false; } });
  const { wrapper, review, save } = await setup();
  review.notes.value = "private unsaved note";
  const keys = [{ sync_pair_id: pairA, doc_id: "doc-0001" }];
  wrapper.getComponent(DocumentList).vm.$emit("export", keys); await flushPromises();
  expect(wrapper.findComponent(ExportDialog).exists()).toBe(false);
  await wrapper.get('[data-testid="leave-stay"]').trigger("click");
  expect(review.notes.value).toBe("private unsaved note");
  wrapper.getComponent(DocumentList).vm.$emit("export", keys); await flushPromises();
  keys[0]!.sync_pair_id = pairB;
  await wrapper.get('[data-testid="leave-save"]').trigger("click"); await flushPromises();
  expect(save).toHaveBeenCalledOnce();
  expect(wrapper.getComponent(ExportDialog).props("keys")).toEqual([{ sync_pair_id: pairA, doc_id: "doc-0001" }]);
  expect(wrapper.findComponent(ReviewView).exists()).toBe(false);
  wrapper.unmount();
});
