import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.vue";
import { pairApi, runApi } from "./lib/ipc";
import { ProcessingConfigSchema } from "./lib/contracts";
import PairManager from "./components/PairManager.vue";
import * as dialog from "@tauri-apps/plugin-dialog";
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), open: vi.fn(), save: vi.fn() }));
afterEach(() => vi.restoreAllMocks());

test("an empty registry offers setup without a pretend sync action", () => {
  const wrapper = mount(App, { props: {
    initialSettings: { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null },
  } });
  expect(wrapper.text()).toContain("Ordnerpaar hinzufügen");
  expect(wrapper.find('[data-testid="start-sync"]').exists()).toBe(false);
});

test("a settings load failure is visible instead of pretending setup is empty", () => {
  const wrapper = mount(App, { props: {
    initialSettings: null,
    initialError: { code: "invalid_settings", retryable: false },
  } });
  expect(wrapper.text()).toContain("Einstellungen konnten nicht geladen werden");
  expect(wrapper.text()).not.toContain("Ordnerpaar hinzufügen");
});

test("a missing source mapping asks for repair without offering empty setup", () => {
  const wrapper = mount(App, { props: {
    initialSettings: null,
    initialError: { code: "mapping_missing", retryable: false },
  } });
  expect(wrapper.text()).toContain("muss repariert werden");
  expect(wrapper.text()).not.toContain("Ordnerpaar hinzufügen");
});

test("a scanned selected pair can run and its controls stay locked until the authoritative finish", async () => {
  const pairId = "11111111-1111-4111-8111-111111111111", runId = "22222222-2222-4222-8222-222222222222";
  vi.spyOn(pairApi, "scanPair").mockResolvedValue({ files: [], errors: [] });
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
  expect(button("Verarbeitung starten").attributes("disabled")).toBeDefined();
  await button("Quellordner einlesen").trigger("click"); await flushPromises();
  await button("Verarbeitung starten").trigger("click"); await flushPromises();
  expect(wrapper.getComponent(PairManager).props("busy")).toBe(true);
  expect(button("Abbrechen").exists()).toBe(true);
  const final = { sync_pair_id: pairId, run_id: runId, stage: "finished" as const, discovered: 0, processed: 0, skipped: 0, failed: 0, unprocessed: 0, warned: 0 };
  getSummary.mockResolvedValue({ ...final, outcome: "completed", errors: [], error: null, audit_warning: true });
  receive(final); await flushPromises();
  expect(wrapper.getComponent(PairManager).props("busy")).toBe(false);
  expect(wrapper.text()).toContain("Protokoll konnte nicht vollständig");
  wrapper.unmount();
});

test.each([false, true])("reprocessing sends exact force IDs only after confirmation=%s", async (accepted) => {
  const pairId = "11111111-1111-4111-8111-111111111111", runId = "22222222-2222-4222-8222-222222222222";
  vi.mocked(dialog.confirm).mockResolvedValue(accepted);
  vi.spyOn(pairApi, "scanPair").mockResolvedValue({ files: [{ doc_id: "doc-0001", relative_path: "reviewed.docx", size_bytes: 5, mtime: null, source_hash_sha256: "a".repeat(64), state: "stale" }], errors: [] });
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
  await button("Quellordner einlesen").trigger("click"); await flushPromises();
  await wrapper.get('input[type="checkbox"]').setValue(true);
  await button("Auswahl erneut verarbeiten").trigger("click"); await flushPromises();
  if (accepted) expect(start).toHaveBeenCalledWith(pairId, ["reviewed.docx"], ["doc-0001"]);
  else expect(start).not.toHaveBeenCalled();
  wrapper.unmount();
});
