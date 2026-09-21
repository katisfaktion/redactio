import { mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { expect, test } from "vitest";
import type { ManagedModel } from "../lib/modelContracts";
import ModelManager from "./ModelManager.vue";

const revision = "0123456789abcdef0123456789abcdef01234567";
const model: ManagedModel = {
  name: `hf:acme/medical-ner@${revision}`,
  version: revision,
  repository: "acme/medical-ner",
  title: "Acme medical NER",
  license: "apache-2.0",
  entity_types: ["DATE", "PERSON"],
  window_tokens: 512,
  download_bytes: 1024,
  installed_bytes: 1024,
  state: "ready",
  catalog_key: null,
  used_by_pairs: [{ id: "11111111-1111-4111-8111-111111111111", name: "Research" }],
  error: null,
};

function mountManager(models: ManagedModel[] = [model]) {
  return mount(ModelManager, {
    props: { models, checked: null, job: null, busy: false, error: null },
    global: { plugins: [createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } })] },
  });
}

test("invalid import URL shows its specific rejection", async () => {
  const wrapper = mountManager([]);
  await wrapper.get('[data-testid="model-url"]').setValue("http://example.invalid/model");
  await wrapper.get("form").trigger("submit");
  expect(wrapper.get('[role="alert"]').text()).toContain("Hugging Face");
  expect(wrapper.emitted("check")).toBeUndefined();
});

test("ready model cannot be downloaded twice", () => {
  const wrapper = mountManager();
  expect(wrapper.text()).toContain("Bereit");
  expect(wrapper.find('[data-testid="download-model"]').exists()).toBe(false);
  expect(wrapper.get('[data-testid="remove-model"]').exists()).toBe(true);
});

test("removal shows the affected pairs", () => {
  const wrapper = mountManager();
  expect(wrapper.text()).toContain("Wird verwendet von: Research");
});

test("repository text is rendered as text", () => {
  const wrapper = mountManager([{ ...model, repository: '<img src=x onerror="alert(1)">' }]);
  expect(wrapper.text()).toContain('<img src=x onerror="alert(1)">');
  expect(wrapper.find("img").exists()).toBe(false);
});
