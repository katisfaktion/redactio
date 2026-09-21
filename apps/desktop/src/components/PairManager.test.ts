import { mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { expect, test, vi } from "vitest";
import type { ModelInfo, Settings } from "../lib/contracts";
import PairManager from "./PairManager.vue";
import * as dialog from "@tauri-apps/plugin-dialog";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), open: vi.fn(), save: vi.fn() }));

const ready = (name: string): ModelInfo => ({ name, version: "a".repeat(40), compatible: true, entity_types: ["PERSON"] });

const pair = {
  id: "11111111-1111-4111-8111-111111111111",
  name: "Akten",
  source_folder: "C:\\Quelle",
  target_folder: "C:\\Ziel",
  created_at: "2026-09-19T10:00:00Z",
  processing_revision: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  config: {
    model: "de_core_news_lg",
    enabled_entities: ["PERSON" as const],
    custom_rules: [],
    include_positions: true,
  },
};

function mountManager(settings: Settings, models: ModelInfo[] = []) {
  return mount(PairManager, {
    props: { settings, models, busy: false },
    global: {
      plugins: [createOnyx({
        i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } },
      })],
    },
  });
}

test("management opens below the selector and keeps draft input when collapsed", async () => {
  const wrapper = mountManager({
    schema_version: 1,
    sync_pairs: [pair],
    selected_sync_pair_id: pair.id,
  });

  expect(wrapper.get('[data-testid="active-pair-select"]').exists()).toBe(true);
  const toggle = wrapper.get('[data-testid="pair-management-toggle"]');
  const management = wrapper.get(`#${toggle.attributes("aria-controls")}`);
  expect(toggle.attributes("aria-expanded")).toBe("false");
  expect(management.attributes("style")).toContain("display: none");
  await toggle.trigger("click");
  expect(toggle.attributes("aria-expanded")).toBe("true");
  expect(management.attributes("style")).not.toContain("display: none");
  await management.get("form input").setValue("Weitere Akten");
  await toggle.trigger("click");
  await toggle.trigger("click");
  expect((management.get("form input").element as HTMLInputElement).value).toBe("Weitere Akten");
  await wrapper.setProps({ busy: true });
  expect(toggle.attributes("disabled")).toBeDefined();
});

test("the initial empty setup remains expanded", () => {
  const wrapper = mountManager({
    schema_version: 1,
    sync_pairs: [],
    selected_sync_pair_id: null,
  });

  expect(wrapper.find("details").exists()).toBe(false);
  expect(wrapper.find('[data-testid="pair-management-toggle"]').exists()).toBe(false);
  expect(wrapper.get("form").text()).toContain("Noch kein Ordnerpaar eingerichtet");
  expect(wrapper.text()).toContain("Quellordner auswählen");
});

test("zero ready models blocks creation and opens model management", async () => {
  const wrapper = mountManager({ schema_version: 1, sync_pairs: [], selected_sync_pair_id: null });
  expect(wrapper.text()).toContain("Installieren Sie zuerst ein Modell");
  expect(wrapper.get('[data-testid="add-pair-submit"]').attributes("disabled")).toBeDefined();
  await wrapper.get('[data-testid="manage-models"]').trigger("click");
  expect(wrapper.emitted("manageModels")).toEqual([[]]);
});

test("one ready model is visibly selected and emitted for creation", async () => {
  const model = ready("hf:fixture/only@" + "a".repeat(40));
  const wrapper = mountManager({ schema_version: 1, sync_pairs: [], selected_sync_pair_id: null }, [model]);
  expect(wrapper.getComponent('[data-testid="creation-model-select"]').props("modelValue")).toBe(model.name);
  vi.mocked(dialog.open).mockResolvedValueOnce("/source").mockResolvedValueOnce("/target");
  await wrapper.get('input[type="text"]').setValue("Akten");
  const choose = wrapper.findAll("button");
  await choose.find(button => button.text() === "Quellordner auswählen")!.trigger("click");
  await choose.find(button => button.text() === "Bestehenden Zielordner auswählen")!.trigger("click");
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("add")!.at(-1)).toEqual(["Akten", "/source", "/target", false, model.name]);
});

test("multiple ready models require an explicit creation choice", async () => {
  const models = [ready("hf:fixture/one@" + "a".repeat(40)), ready("hf:fixture/two@" + "b".repeat(40))];
  const wrapper = mountManager({ schema_version: 1, sync_pairs: [], selected_sync_pair_id: null }, models);
  expect(wrapper.getComponent('[data-testid="creation-model-select"]').props("modelValue")).toBeUndefined();
  expect(wrapper.get('[data-testid="add-pair-submit"]').attributes("disabled")).toBeDefined();
  await wrapper.get(`[role="option"][aria-label="${models[1]!.name} (${models[1]!.version.slice(0, 7)})"]`).trigger("click");
  expect(wrapper.getComponent('[data-testid="creation-model-select"]').props("modelValue")).toBe(models[1]!.name);
});
