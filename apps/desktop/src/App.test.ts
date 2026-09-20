import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.vue";
import { pairApi, runApi, detectionApi, reviewApi } from "./lib/ipc";
import { ProcessingConfigSchema, type ReviewViewData } from "./lib/contracts";
import PairManager from "./components/PairManager.vue";
import DocumentList from "./components/DocumentList.vue";
import * as dialog from "@tauri-apps/plugin-dialog";
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), open: vi.fn(), save: vi.fn() }));
afterEach(() => vi.restoreAllMocks());

test("an empty registry offers setup without a pretend sync action", () => {
  const wrapper = mount(App, { props: {
    initialSettings: { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null },
  } });
  expect(wrapper.text()).toContain("Ordnerpaar hinzufügen");
  expect(wrapper.find('[data-testid="start-sync"]').exists()).toBe(false);
  expect(wrapper.findAll("main")).toHaveLength(1);
});

test("a settings load failure is visible instead of pretending setup is empty", () => {
  const wrapper = mount(App, { props: {
    initialSettings: null,
    initialError: { code: "invalid_settings", retryable: false },
  } });
  expect(wrapper.text()).toContain("Einstellungen konnten nicht geladen werden");
  expect(wrapper.text()).not.toContain("Ordnerpaar hinzufügen");
});

test("mobile navigation closes after changing views and after selecting the current view", async () => {
  vi.stubGlobal("ResizeObserver", class {
    constructor(private callback: ResizeObserverCallback) {}
    observe(target: Element) {
      if (target.classList.contains("onyx-nav-bar")) this.callback([
        { target, contentBoxSize: [{ inlineSize: 520, blockSize: 64 }] } as ResizeObserverEntry,
      ], this as unknown as ResizeObserver);
    }
    unobserve() {}
    disconnect() {}
  });
  vi.spyOn(runApi, "auditLocation").mockResolvedValue("/config/audit-log.jsonl");
  const wrapper = mount(App, { props: {
    initialSettings: { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null },
  } });
  try {
    await flushPromises();
    const burger = () => wrapper.get(".onyx-nav-bar__burger button");
    const menu = () => wrapper.get(".onyx-mobile-nav-button__flyout");
    await burger().trigger("click");
    expect(menu().attributes("style")).not.toContain("display: none");
    await wrapper.get('[data-testid="settings-nav"]').trigger("click");
    await flushPromises();
    expect(wrapper.get("h1").text()).toBe("Einstellungen");
    expect(menu().attributes("style")).toContain("display: none");
    await burger().trigger("click");
    await wrapper.get('[data-testid="settings-nav"]').trigger("click");
    await flushPromises();
    expect(menu().attributes("style")).toContain("display: none");
  } finally {
    wrapper.unmount();
    vi.unstubAllGlobals();
  }
});

test("a missing source mapping asks for repair without offering empty setup", () => {
  const wrapper = mount(App, { props: {
    initialSettings: null,
    initialError: { code: "mapping_missing", retryable: false },
  } });
  expect(wrapper.text()).toContain("muss repariert werden");
  expect(wrapper.text()).not.toContain("Ordnerpaar hinzufügen");
});

test("a scanned selected pair stays locked through the authoritative finish and list refresh", async () => {
  const pairId = "11111111-1111-4111-8111-111111111111", runId = "22222222-2222-4222-8222-222222222222";
  let finishRefresh!: (report: Awaited<ReturnType<typeof pairApi.scanPair>>) => void;
  const refresh = new Promise<Awaited<ReturnType<typeof pairApi.scanPair>>>(resolve => { finishRefresh = resolve; });
  const scan = vi.spyOn(pairApi, "scanPair").mockResolvedValueOnce({ files: [], errors: [] }).mockReturnValueOnce(refresh);
  let receive!: (payload: unknown) => void;
  vi.spyOn(runApi, "listen").mockImplementation(async (callback) => { receive = callback; return () => {}; });
  vi.spyOn(runApi, "start").mockResolvedValue(runId);
  const getSummary = vi.spyOn(runApi, "summary").mockResolvedValue(null);
  const wrapper = mount(App, { props: { initialSettings: {
    schema_version: 1, selected_sync_pair_id: pairId, sync_pairs: [{
      id: pairId, name: "Sammlung", source_folder: "/source", target_folder: "/target", created_at: "2026-09-19T10:00:00Z", processing_revision: runId,
      config: ProcessingConfigSchema.parse({ model: "de_core_news_lg", enabled_entities: [], custom_rules: [], include_positions: true }),
    }],
  } } });
  const button = (label: string) => wrapper.findAll("button").find((item) => item.text().includes(label))!;
  await flushPromises();
  expect(button("Verarbeitung starten").attributes("disabled")).toBeDefined();
  await button("Quellordner einlesen").trigger("click"); await flushPromises();
  await button("Verarbeitung starten").trigger("click"); await flushPromises();
  expect(wrapper.getComponent(PairManager).props("busy")).toBe(true);
  expect(wrapper.get('[data-testid="settings-nav"]').attributes("disabled")).toBeDefined();
  expect(button("Abbrechen").exists()).toBe(true);
  const final = { sync_pair_id: pairId, run_id: runId, stage: "finished" as const, discovered: 1, processed: 1, skipped: 0, failed: 0, unprocessed: 0, warned: 0 };
  getSummary.mockResolvedValue({ ...final, outcome: "completed", errors: [], error: null, audit_warning: true });
  receive(final); await flushPromises();
  expect(wrapper.getComponent(PairManager).props("busy")).toBe(true);
  expect(wrapper.text()).toContain("Protokoll konnte nicht vollständig");
  expect(button("Verarbeitung starten").attributes("disabled")).toBeDefined();
  expect(wrapper.text()).not.toContain("0 Dokumente gefunden");
  expect(scan).toHaveBeenCalledTimes(2);

  finishRefresh({ files: [{ doc_id: "doc-0001", relative_path: "processed.docx", size_bytes: 5, mtime: null,
    source_hash_sha256: "a".repeat(64), review_status: "pending", state: "current" }], errors: [] });
  await flushPromises();
  expect(wrapper.getComponent(PairManager).props("busy")).toBe(false);
  expect(wrapper.get('[data-testid="settings-nav"]').attributes("disabled")).toBeUndefined();
  expect(wrapper.text()).toContain("processed.docx");
  expect(button("Verarbeitung starten").attributes("disabled")).toBeUndefined();
  const reviewData: ReviewViewData = {
    key: { sync_pair_id: pairId, doc_id: "doc-0001" }, source_hash: "a".repeat(64), revision: runId,
    expected_output_hash: "b".repeat(64), expected_review_hash: "c".repeat(64), original_text: "Anna", markdown: "Anna", body: "Anna",
    detections: [], redactions: [], decisions: { dismissed_ids: [], manual: [] }, warnings: [], acknowledged_warnings: [], notes: "", status: "pending",
  };
  const open = vi.spyOn(reviewApi, "open").mockResolvedValue(reviewData);
  await wrapper.get('[data-testid="review-doc-0001"]').trigger("click");
  await flushPromises();
  expect(open).toHaveBeenCalledWith({ sync_pair_id: pairId, doc_id: "doc-0001" });
  expect(wrapper.text()).toContain("Dokument prüfen");
  wrapper.unmount();
});

test.each([false, true])("reprocessing sends exact force IDs only after confirmation=%s", async (accepted) => {
  const pairId = "11111111-1111-4111-8111-111111111111", runId = "22222222-2222-4222-8222-222222222222";
  vi.mocked(dialog.confirm).mockResolvedValue(accepted);
  vi.spyOn(pairApi, "scanPair").mockResolvedValue({ files: [{ doc_id: "doc-0001", relative_path: "reviewed.docx", size_bytes: 5, mtime: null, source_hash_sha256: "a".repeat(64), review_status: null, state: "stale" }], errors: [] });
  vi.spyOn(runApi, "listen").mockResolvedValue(() => {});
  const start = vi.spyOn(runApi, "start").mockResolvedValue(runId);
  vi.spyOn(runApi, "summary").mockResolvedValue(null);
  const wrapper = mount(App, { props: { initialSettings: {
    schema_version: 1, selected_sync_pair_id: pairId, sync_pairs: [{
      id: pairId, name: "Sammlung", source_folder: "/source", target_folder: "/target", created_at: "2026-09-19T10:00:00Z", processing_revision: runId,
      config: { model: "de_core_news_lg", enabled_entities: [], custom_rules: [], include_positions: true },
    }],
  } } });
  const button = (label: string) => wrapper.findAll("button").find((item) => item.text().includes(label))!;
  await flushPromises();
  await button("Quellordner einlesen").trigger("click"); await flushPromises();
  await wrapper.get('input[type="checkbox"]').setValue(true);
  await button("Auswahl erneut verarbeiten").trigger("click"); await flushPromises();
  if (accepted) expect(start).toHaveBeenCalledWith(pairId, ["reviewed.docx"], ["doc-0001"]);
  else expect(start).not.toHaveBeenCalled();
  wrapper.unmount();
});

test("saving detection settings retains the pair and clears documents from the old processing revision", async () => {
  const pairId = "11111111-1111-4111-8111-111111111111";
  const initial = { schema_version: 1 as const, selected_sync_pair_id: pairId, sync_pairs: [{
    id: pairId, name: "Sammlung", source_folder: "/source", target_folder: "/target", created_at: "2026-09-19T10:00:00Z", processing_revision: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    config: { model: "de_core_news_lg", enabled_entities: [], custom_rules: [], include_positions: true },
  }] };
  vi.spyOn(detectionApi,"listModels").mockResolvedValue([{name:"de_core_news_lg",version:"3.8.0",compatible:true}]);
  vi.spyOn(detectionApi,"refresh").mockResolvedValue(initial);
  const save = vi.spyOn(detectionApi,"save").mockImplementation(async (_,config)=>({...initial,sync_pairs:[{...initial.sync_pairs[0]!,config,processing_revision:"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"}]}));
  vi.spyOn(runApi,"auditLocation").mockResolvedValue("/config/audit-log.jsonl");
  vi.spyOn(runApi,"listen").mockResolvedValue(()=>{});
  vi.spyOn(reviewApi,"open").mockRejectedValue({ code: "review_mismatch", retryable: false });
  const wrapper=mount(App,{props:{initialSettings:initial},attachTo:document.body}); await flushPromises();
  expect(wrapper.find('[data-testid="document-view"]').isVisible()).toBe(true);
  expect(wrapper.get('[data-testid="settings-nav"]').attributes("disabled")).toBeUndefined();
  wrapper.getComponent(DocumentList).vm.$emit("scanned", { files: [{ doc_id: "doc-old", relative_path: "old.docx", size_bytes: 5, mtime: null,
    source_hash_sha256: "a".repeat(64), review_status: "pending", state: "current" }], errors: [] });
  wrapper.getComponent(DocumentList).vm.$emit("review", "doc-old"); await flushPromises();
  expect(wrapper.getComponent('[data-testid="review-document-select"]').props("options")).toHaveLength(1);
  await wrapper.get('[data-testid="settings-nav"]').trigger("click"); await flushPromises();
  expect(wrapper.get('[data-testid="settings-nav"]').attributes("aria-current")).toBe("page");
  expect(wrapper.get('[data-testid="document-view"]').attributes("style")).toContain("display: none");
  expect(wrapper.get('[data-testid="active-pair-select"]').isVisible()).toBe(true);
  expect(wrapper.find('[data-testid="document-view"]').isVisible()).toBe(false);
  await wrapper.get('[data-testid="include-positions"]').setValue(false);
  await wrapper.get('.detection-settings form').trigger("submit"); await flushPromises();
  expect(save).toHaveBeenCalledWith(pairId,{...initial.sync_pairs[0]!.config,include_positions:false});
  expect(wrapper.text()).toContain("Erkennung gespeichert");
  await wrapper.get('[data-testid="documents-nav"]').trigger("click");
  expect(wrapper.get('[data-testid="active-pair-select"]').isVisible()).toBe(true);
  expect(wrapper.find('[data-testid="document-view"]').isVisible()).toBe(true);
  wrapper.getComponent(DocumentList).vm.$emit("review", "doc-old"); await flushPromises();
  expect(wrapper.getComponent('[data-testid="review-document-select"]').props("options")).toEqual([]);
  wrapper.unmount();
});
