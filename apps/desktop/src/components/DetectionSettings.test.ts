import { mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { expect, test } from "vitest";
import type { SyncPair } from "../lib/contracts";
import DetectionSettings from "./DetectionSettings.vue";

const pair: SyncPair = {
  id: "11111111-1111-4111-8111-111111111111", name: "Akten A",
  source_folder: "C:\\A", target_folder: "C:\\B", created_at: "2026-09-19T10:00:00Z",
  processing_revision: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  config: { model: "de_core_news_lg", enabled_entities: ["PERSON"], custom_rules: [], include_positions: true },
};
const models = [
  { name: "de_core_news_lg", version: "3.8.0", compatible: true },
  { name: "de_core_news_sm", version: "3.8.0", compatible: false },
];

function setup(selected = pair) {
  return mount(DetectionSettings, {
    props: { pair: selected, models, busy: false },
    global: { plugins: [createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } })] },
  });
}

test("edits a private draft and emits complete pair-bound save and preview snapshots", async () => {
  const wrapper = setup();
  await wrapper.get('[data-testid="entity-PERSON"]').setValue(false);
  await wrapper.get('[data-testid="include-positions"]').setValue(false);
  await wrapper.get('[data-testid="add-regex"]').trigger("click");
  expect(wrapper.get('[data-testid="save"]').attributes("disabled")).toBeDefined();
  await wrapper.get('[data-testid="rule-pattern"]').setValue("(?i)Anna");
  await wrapper.get('[data-testid="add-words"]').trigger("click");
  await wrapper.get('[data-testid="rule-words"]').setValue(" Anna \nBeispiel");
  await wrapper.get('[data-testid="preview-text"]').setValue("Anna Beispiel");
  await wrapper.get('[data-testid="preview"]').trigger("click");
  await wrapper.get("form").trigger("submit");
  const [id, config] = wrapper.emitted("save")![0]!;
  expect(id).toBe(pair.id);
  expect(config).toMatchObject({ model: "de_core_news_lg", enabled_entities: [], include_positions: false,
    custom_rules: [
      { kind: "regex", pattern: "(?i)Anna", entity_type: "CUSTOM", enabled: true },
      { kind: "words", words: [" Anna ", "Beispiel"], entity_type: "CUSTOM", enabled: true },
    ],
  });
  expect(wrapper.emitted("preview")![0]).toEqual([pair.id, config, "Anna Beispiel"]);
  const rules = (config as SyncPair["config"]).custom_rules;
  expect(rules[0]!.id).not.toBe(rules[1]!.id);
  expect(pair.config.custom_rules).toEqual([]);
  expect(pair.config.enabled_entities).toEqual(["PERSON"]);
  await wrapper.get('[data-testid="rule-pattern"]').setValue("Changed");
  expect(config).toMatchObject({ custom_rules: [{ pattern: "(?i)Anna" }, {}] });
});

test("model and rule controls retain their selected values in the emitted configuration", async () => {
  const wrapper = setup();
  await wrapper.setProps({ models: [...models, { name: "de_core_news_md", version: "3.8.0", compatible: true }] });
  await wrapper.get('[role="option"][aria-label="de_core_news_md (3.8.0)"]').trigger("click");
  await wrapper.get('[role="option"][aria-disabled="true"]').trigger("click");
  await wrapper.get('[data-testid="add-regex"]').trigger("click");
  await wrapper.get('[data-testid="rule-pattern"]').setValue("Anna");
  await wrapper.get('[role="listbox"][aria-label="Entitätstypen"] [role="option"][aria-label="Personen"]').trigger("click");
  const ruleField = wrapper.findAll("fieldset")[1]!;
  await ruleField.get('input[type="checkbox"]').setValue(false);
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")![0]![1]).toMatchObject({
    model: "de_core_news_md", custom_rules: [{ pattern: "Anna", entity_type: "PERSON", enabled: false }],
  });
});

test("rejects empty entries and unavailable models and blocks actions while busy", async () => {
  const wrapper = setup({ ...pair, config: { ...pair.config, model: "missing_model" } });
  expect(wrapper.text()).toContain("nicht verfügbar oder nicht kompatibel");
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")).toBeUndefined();
  await wrapper.setProps({ pair });
  await wrapper.get('[data-testid="add-words"]').trigger("click");
  await wrapper.get('[data-testid="rule-words"]').setValue("Anna\n\nBeispiel");
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")).toBeUndefined();
  expect(wrapper.text()).toContain("leere Einträge");
  await wrapper.get('[data-testid="remove-rule"]').trigger("click");
  await wrapper.setProps({ busy: true });
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")).toBeUndefined();
  expect(wrapper.get('[data-testid="preview"]').attributes("disabled")).toBeDefined();
});

test("switching pairs clears the private preview and draft rules", async () => {
  const wrapper = setup();
  await wrapper.get('[data-testid="add-regex"]').trigger("click");
  await wrapper.get('[data-testid="preview-text"]').setValue("private preview");
  const other = { ...pair, id: "22222222-2222-4222-8222-222222222222", name: "Akten B" };
  await wrapper.setProps({ pair: other });
  expect(wrapper.find('[data-testid="rule-pattern"]').exists()).toBe(false);
  expect((wrapper.get('[data-testid="preview-text"]').element as HTMLTextAreaElement).value).toBe("");
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")![0]).toEqual([other.id, other.config]);
});

test("preview uses Unicode code points, escapes text and disappears after draft changes", async () => {
  const wrapper = setup();
  const text = "😀 <b>Anna</b>";
  await wrapper.get('[data-testid="preview-text"]').setValue(text);
  await wrapper.setProps({ preview: { pairId: pair.id, config: pair.config, text,
    detections: [{ id: "1", start: 2, end: 13, entity_type: "CUSTOM", confidence: 1, recognizer: "synthetic", origin: "automatic" }],
  } });
  expect(wrapper.get('[data-testid="preview-results"]').text()).toContain("<b>Anna</b>");
  expect(wrapper.find('[data-testid="preview-results"] b').exists()).toBe(false);
  await wrapper.get('[data-testid="include-positions"]').setValue(false);
  expect(wrapper.find('[data-testid="preview-results"]').exists()).toBe(false);
  await wrapper.setProps({ error: { code: "engine_timeout", retryable: false } });
  expect(wrapper.text()).toContain("Zeitlimit");
});
