import { mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { expect, test } from "vitest";
import { OnyxInput } from "sit-onyx";
import type { ManagedModel } from "../lib/modelContracts";
import { checked, managed } from "../test/modelFixture";
import ModelManager from "./ModelManager.vue";

const model = managed();

function mountManager(models: ManagedModel[] = [model], staleCheck = null) {
  return mount(ModelManager, {
    props: { models, checked: staleCheck, job: null, busy: false, error: null },
    global: { plugins: [createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } })] },
  });
}

test("invalid import URL shows its specific rejection", async () => {
  const wrapper = mountManager([]);
  await wrapper.get('[data-testid="model-url"]').setValue("http://example.invalid/model");
  await wrapper.get("form").trigger("submit");
  expect(wrapper.find('[role="alert"]').exists()).toBe(false);
  expect(wrapper.getComponent(OnyxInput).props("error")).toContain("Hugging Face");
  expect(wrapper.emitted("check")).toBeUndefined();
});

test("ready model cannot be downloaded twice", () => {
  const wrapper = mountManager([model], checked({ model: managed({ state: "available", installed_bytes: 0 }) }));
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
