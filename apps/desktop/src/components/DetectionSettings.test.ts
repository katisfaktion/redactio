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
  config: { model: "OpenMed-PII-German-BiomedBERT-Large-340M-v1", model_entities: null, enabled_entities: ["PERSON"], custom_rules: [], include_positions: true },
};
const models = [
  { name: "OpenMed-PII-German-BiomedBERT-Large-340M-v1", version: "ce797d58600cc20bba9a2500dafc0b7f5c3270c1", compatible: true, entity_types: ["LOCATION", "PERSON"] },
  { name: "unavailable", version: "3.8.0", compatible: false, entity_types: [] },
];

function setup(selected = pair) {
  return mount(DetectionSettings, {
    props: { pair: selected, models, busy: false },
    global: { plugins: [createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } })] },
  });
}

test("edits a private draft and emits complete pair-bound save and preview snapshots", async () => {
  const wrapper = setup();
  await wrapper.get('[data-testid="model-entity-PERSON"]').setValue(false);
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
  expect(config).toMatchObject({ model: "OpenMed-PII-German-BiomedBERT-Large-340M-v1", model_entities: [], enabled_entities: [], include_positions: false,
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

test("model controls use each model's metadata and preserve a draft per model", async () => {
  const wrapper = setup();
  await wrapper.setProps({ models: [...models, { name: "synthetic-54", version: "1", compatible: true, entity_types: ["ALPHA", "BILLING_ACCOUNT", ...Array.from({ length: 52 }, (_, index) => `LABEL_${index}`)] }] });
  await wrapper.get('[role="option"][aria-label="synthetic-54 (1)"]').trigger("click");
  expect(wrapper.findAll('[data-testid^="model-entity-"]')).toHaveLength(54);
  expect(wrapper.text()).toContain("BILLING_ACCOUNT");
  await wrapper.get('[data-testid="model-entity-ALPHA"]').setValue(false);
  expect(wrapper.get('[data-testid="model-entity-BILLING_ACCOUNT"]').element.closest("label")?.querySelector("p")?.className)
    .toContain("onyx-truncation-multiline");
  await wrapper.get('[role="option"][aria-label="BiomedBERT – Deutsch, PII (340M, ce797d5)"]').trigger("click");
  await wrapper.get('[role="option"][aria-label="synthetic-54 (1)"]').trigger("click");
  expect(wrapper.get('[data-testid="model-entity-ALPHA"]').attributes("checked")).toBeUndefined();
  await wrapper.get('[role="option"][aria-disabled="true"]').trigger("click");
  await wrapper.get('[data-testid="add-regex"]').trigger("click");
  await wrapper.get('[data-testid="rule-pattern"]').setValue("Anna");
  await wrapper.get('[role="listbox"][aria-label="Entitätstypen"] [role="option"][aria-label="Person (PERSON)"]')
    .trigger("click");
  const ruleField = wrapper.findAll("fieldset").at(-1)!;
  await ruleField.get('input[type="checkbox"]').setValue(false);
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")![0]![1]).toMatchObject({
    model: "synthetic-54", model_entities: expect.arrayContaining(["BILLING_ACCOUNT"]),
    custom_rules: [{ pattern: "Anna", entity_type: "PERSON", enabled: false }],
  });
});

test("selects BiomedBERT by its exact local model identity", async () => {
  const name = "OpenMed-PII-German-BiomedBERT-Large-340M-v1";
  const wrapper = setup();
  await wrapper.setProps({ models: [...models, {
    name, version: "ce797d58600cc20bba9a2500dafc0b7f5c3270c1", compatible: true, entity_types: ["PERSON", "LOCATION"],
  }] });
  await wrapper.get('[role="option"][aria-label="BiomedBERT – Deutsch, PII (340M, ce797d5)"]').trigger("click");
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")![0]![1]).toMatchObject({ model: name });
  expect(wrapper.text()).toContain("lokal auf der CPU");
  expect(wrapper.text()).toContain("Person");
});

test("legacy drafts disable every native name and address label with its legacy group", async () => {
  const model = { name: "native", version: "1", compatible: true,
    entity_types: ["AGE", "FIRSTNAME", "MIDDLENAME", "LASTNAME", "STREET", "BUILDINGNUMBER", "SECONDARYADDRESS", "ZIPCODE", "CITY", "STATE", "COUNTY", "GPSCOORDINATES", "ORDINALDIRECTION"] };
  const legacy = { ...pair, config: { ...pair.config, model: "native", model_entities: null, enabled_entities: [] } };
  const wrapper = mount(DetectionSettings, {
    props: { pair: legacy, models: [model], busy: false },
    global: { plugins: [createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } })] },
  });
  expect((wrapper.get('[data-testid="model-entity-AGE"]').element as HTMLInputElement).checked).toBe(true);
  for (const type of ["FIRSTNAME", "MIDDLENAME", "LASTNAME", "STREET", "BUILDINGNUMBER", "SECONDARYADDRESS", "ZIPCODE", "CITY", "STATE", "COUNTY", "GPSCOORDINATES", "ORDINALDIRECTION"]) {
    expect((wrapper.get(`[data-testid="model-entity-${type}"]`).element as HTMLInputElement).checked).toBe(false);
  }
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")![0]![1]).toMatchObject({ model_entities: ["AGE"], enabled_entities: [] });
});

test("switching an unavailable legacy pair strips legacy entities from supplementary recognizers", async () => {
  const legacy = { ...pair, config: { ...pair.config, model: "de_core_news_lg", model_entities: null,
    enabled_entities: ["PERSON", "LOCATION", "EMAIL_ADDRESS"] } };
  const wrapper = setup(legacy);
  await wrapper.get('[role="option"][aria-label="BiomedBERT – Deutsch, PII (340M, ce797d5)"]').trigger("click");
  await wrapper.get("form").trigger("submit");
  expect(wrapper.emitted("save")![0]![1]).toMatchObject({
    model: "OpenMed-PII-German-BiomedBERT-Large-340M-v1", model_entities: ["LOCATION", "PERSON"],
    enabled_entities: ["EMAIL_ADDRESS"],
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
  expect(wrapper.emitted("save")![0]).toEqual([other.id, { ...other.config, model_entities: ["PERSON"], enabled_entities: [] }]);
});

test("preview uses Unicode code points, escapes text and disappears after draft changes", async () => {
  const wrapper = setup();
  const text = "😀 <b>Anna</b>";
  await wrapper.get('[data-testid="preview-text"]').setValue(text);
  await wrapper.setProps({ preview: { pairId: pair.id, config: { ...pair.config, model_entities: ["PERSON"], enabled_entities: [] }, text,
    detections: [{ id: "1", start: 2, end: 13, entity_type: "CUSTOM", confidence: 1, recognizer: "synthetic", origin: "automatic" }],
  } });
  expect(wrapper.get('[data-testid="preview-results"]').text()).toContain("<b>Anna</b>");
  expect(wrapper.find('[data-testid="preview-results"] b').exists()).toBe(false);
  await wrapper.get('[data-testid="include-positions"]').setValue(false);
  expect(wrapper.find('[data-testid="preview-results"]').exists()).toBe(false);
  await wrapper.setProps({ error: { code: "engine_timeout", retryable: false } });
  expect(wrapper.text()).toContain("Zeitlimit");
});
