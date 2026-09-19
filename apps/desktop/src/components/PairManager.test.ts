import { mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { expect, test } from "vitest";
import type { Settings } from "../lib/contracts";
import PairManager from "./PairManager.vue";

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

function mountManager(settings: Settings) {
  return mount(PairManager, {
    props: { settings, busy: false },
    global: {
      plugins: [createOnyx({
        i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } },
      })],
    },
  });
}

test("existing pair management is collapsed while the active selector stays visible", () => {
  const wrapper = mountManager({
    schema_version: 1,
    sync_pairs: [pair],
    selected_sync_pair_id: pair.id,
  });

  expect(wrapper.get('[data-testid="active-pair-select"]').exists()).toBe(true);
  const management = wrapper.get("details");
  expect(management.attributes("open")).toBeUndefined();
  expect(management.get("summary").text()).toContain("Ordnerpaare verwalten");
  expect(management.find("form").exists()).toBe(true);
});

test("the initial empty setup remains expanded", () => {
  const wrapper = mountManager({
    schema_version: 1,
    sync_pairs: [],
    selected_sync_pair_id: null,
  });

  expect(wrapper.find("details").exists()).toBe(false);
  expect(wrapper.get("form").text()).toContain("Noch kein Ordnerpaar eingerichtet");
  expect(wrapper.text()).toContain("Quellordner auswählen");
});
