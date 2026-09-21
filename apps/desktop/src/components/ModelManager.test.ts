import { mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { expect, test } from "vitest";
import { OnyxInput } from "sit-onyx";
import type { SafeError } from "../lib/contracts";
import type { CheckedModel, ManagedModel, ModelJob } from "../lib/modelContracts";
import { checked, job, managed } from "../test/modelFixture";
import ModelManager from "./ModelManager.vue";

const model = managed();

function mountManager(
  models: ManagedModel[] = [model],
  options: { checked?: CheckedModel | null; job?: ModelJob | null; busy?: boolean; error?: SafeError | null } = {},
) {
  return mount(ModelManager, {
    props: {
      models,
      checked: options.checked ?? null,
      job: options.job ?? null,
      busy: options.busy ?? false,
      error: options.error ?? null,
    },
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
  const wrapper = mountManager([model], { checked: checked({ model: managed({ state: "available", installed_bytes: 0 }) }) });
  expect(wrapper.text()).toContain("Bereit");
  expect(wrapper.find('[data-testid="download-model"]').exists()).toBe(false);
  expect(wrapper.get('[data-testid="remove-model"]').exists()).toBe(true);
});

test("checked imports show verified provenance before download", () => {
  const wrapper = mountManager([], { checked: checked() });
  const result = wrapper.get('[data-testid="checked-model"]');

  expect(result.text()).toContain("0123456789abcdef0123456789abcdef01234567");
  expect(result.text()).toContain("1 kB");
  expect(result.text()).toContain("DATE, PERSON");
  expect(result.text()).toContain("apache-2.0");
  expect(result.get('[data-testid="model-source"]').attributes("href")).toBe("https://huggingface.co/acme/medical-ner");
  expect(result.get('[data-testid="model-license"]').attributes("href")).toBe("https://huggingface.co/acme/medical-ner");
});

test("available retained imports can be checked from their receipt", async () => {
  const availableReceipt = managed({ state: "available", installed_bytes: 0, used_by_pairs: [] });
  const wrapper = mountManager([availableReceipt]);

  await wrapper.get('[data-testid="check-receipt"]').trigger("click");
  expect(wrapper.emitted("check")).toEqual([[{ kind: "receipt", name: availableReceipt.name }]]);
});

test("invalid receipts show a repair action and a German compatibility reason", async () => {
  const invalid = managed({
    state: "invalid",
    installed_bytes: 0,
    used_by_pairs: [],
    error: { code: "model_incompatible", retryable: false },
  });
  const wrapper = mountManager([invalid]);

  expect(wrapper.text()).toContain("nicht kompatibel");
  expect(wrapper.text()).toContain("Noch herunterzuladen: 1 kB");
  await wrapper.get('[data-testid="retry-receipt"]').trigger("click");
  expect(wrapper.emitted("check")).toEqual([[{ kind: "receipt", name: invalid.name }]]);
});

test("invalid installed entries can be removed unless a saved pair uses them", async () => {
  const invalid = managed({ state: "invalid", installed_bytes: 1_024, used_by_pairs: [] });
  const wrapper = mountManager([invalid]);
  expect(wrapper.text()).toContain("Belegt: 1 kB");
  await wrapper.get('[data-testid="remove-model"]').trigger("click");
  expect(wrapper.emitted("remove")).toEqual([[invalid.name]]);

  await wrapper.setProps({ models: [managed({ state: "invalid", installed_bytes: 1_024 })] });
  expect(wrapper.get('[data-testid="remove-model"]').attributes("disabled")).toBeDefined();
  expect(wrapper.text()).toContain("Entfernen erst möglich");
});

test.each([
  ["model_plan_expired", "erneut"], ["invalid_model_protocol", "neu"],
  ["model_worker_unavailable", "neu"], ["model_install_interrupted", "erneut"],
  ["operation_busy", "warten"], ["operation_cancelled", "abgebrochen"],
  ["state_unavailable", "neu"],
])("host error %s has an actionable German message", (code, action) => {
  const wrapper = mountManager([], { error: { code, retryable: true } });
  expect(wrapper.get('[role="alert"]').text().toLocaleLowerCase("de-DE")).toContain(action);
});

test("interrupted removal can resume with the existing removal command", async () => {
  const removing = managed({ state: "removing", download_bytes: 2_000, used_by_pairs: [] });
  const wrapper = mountManager([removing]);

  expect(wrapper.text()).toContain("wurde unterbrochen");
  expect(wrapper.text()).toContain("Belegt: 1 kB");
  await wrapper.get('[data-testid="resume-removal"]').trigger("click");
  expect(wrapper.emitted("remove")).toEqual([[removing.name]]);
});

test("removal is blocked while affected pairs still use the model", () => {
  const wrapper = mountManager();
  expect(wrapper.text()).toContain("Wird verwendet von: Research");
  expect(wrapper.text()).toContain("Entfernen erst möglich");
  expect(wrapper.text()).toContain("Belegt: 1 kB");
  expect(wrapper.get('[data-testid="remove-model"]').attributes("disabled")).toBeDefined();
});

test("read-only storage gives the actionable whole-app-folder instruction", () => {
  const wrapper = mountManager([], { error: { code: "model_store_read_only", retryable: true } });
  expect(wrapper.get('[role="alert"]').text()).toContain("gesamten App-Ordner");
});

test("cancel remains disabled after a terminal job", () => {
  const wrapper = mountManager([], { job: job({ stage: "failed" }), busy: true });
  expect(wrapper.get('[data-testid="cancel-job"]').attributes("disabled")).toBeDefined();
});

test("repository text is rendered as text", () => {
  const wrapper = mountManager([{ ...model, repository: '<img src=x onerror="alert(1)">' }]);
  expect(wrapper.text()).toContain('<img src=x onerror="alert(1)">');
  expect(wrapper.find("img").exists()).toBe(false);
});
