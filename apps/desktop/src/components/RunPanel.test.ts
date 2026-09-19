import { mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { expect, test } from "vitest";
import RunPanel from "./RunPanel.vue";

const counts = { discovered: 4, processed: 1, skipped: 1, failed: 1, unprocessed: 1, warned: 1 };

function mountPanel() {
  return mount(RunPanel, {
    props: { pairName: "Sammlung A", counts, stage: "processing" },
    global: { plugins: [createOnyx({
      i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } },
    })] },
  });
}

test("progress counts each discovered document once and labels warning counts as a subset", () => {
  const panel = mountPanel();
  expect(panel.get("h2").text()).toContain("Sammlung A");
  expect(panel.get("progress").attributes()).toMatchObject({ value: "3", max: "4" });
  expect(panel.get('[role="status"]').text()).toContain("3 von 4");
  expect(panel.text()).toContain("Davon mit Warnungen: 1");
  for (const label of ["Verarbeitet: 1", "Übersprungen: 1", "Fehlgeschlagen: 1", "Nicht verarbeitet: 1"]) {
    expect(panel.text()).toContain(label);
  }
});

test("initializing and scanning announce their stage without a misleading zero denominator", async () => {
  const panel = mountPanel();
  await panel.setProps({ stage: "initializing", counts: { discovered: 0, processed: 0, skipped: 0, failed: 0, unprocessed: 0, warned: 0 } });
  expect(panel.get('[role="status"]').text()).toContain("Vorbereitung");
  expect(panel.get("progress").attributes("value")).toBeUndefined();
  await panel.setProps({ stage: "scanning" });
  expect(panel.get('[role="status"]').text()).toContain("Quellordner");
  expect(panel.get("progress").attributes("value")).toBeUndefined();
});

test.each(["initializing", "scanning", "processing"] as const)("cancel during %s emits once while its pending state explains the bounded wait", async (stage) => {
  const panel = mountPanel();
  await panel.setProps({ stage });
  await panel.get("button").trigger("click");
  expect(panel.emitted("cancel")).toHaveLength(1);
  await panel.setProps({ cancelling: true });
  expect(panel.get("button").attributes("disabled")).toBeDefined();
  expect(panel.get('[role="status"]').text()).toContain("Abbruch angefordert");
  await panel.get("button").trigger("click");
  expect(panel.emitted("cancel")).toHaveLength(1);
});

test.each([
  ["completed", "Abgeschlossen"],
  ["completed-with-errors", "Mit Fehlern abgeschlossen"],
  ["cancelled", "Abgebrochen"],
  ["failed", "Verarbeitung fehlgeschlagen"],
] as const)("finished %s is distinguishable and allows another run", async (outcome, label) => {
  const panel = mountPanel();
  await panel.setProps({ stage: "finished", outcome });
  expect(panel.get('[role="status"]').text()).toContain(label);
  expect(panel.get("button").text()).toContain("Verarbeitung starten");
  await panel.get("button").trigger("click");
  expect(panel.emitted("start")).toHaveLength(1);
  expect(panel.emitted("cancel")).toBeUndefined();
});

test("local failure paths are escaped and audit failure remains visible beside results", async () => {
  const panel = mountPanel();
  await panel.setProps({
    stage: "finished", outcome: "completed-with-errors", auditWarning: true,
    errors: [{ relative_path: "<img src=x onerror=alert(1)>.docx", code: "invalid_docx", retryable: false }],
    error: { code: "private_error_canary", retryable: false },
  });
  expect(panel.text()).toContain("<img src=x onerror=alert(1)>.docx");
  expect(panel.find("img").exists()).toBe(false);
  expect(panel.text()).not.toContain("private_error_canary");
  expect(panel.text()).toContain("Verarbeitet: 1");
  expect(panel.findAll('[role="alert"]').map((alert) => alert.text()).join(" ")).toContain("Die Verarbeitung konnte nicht durchgeführt werden");
  expect(panel.findAll('[role="alert"]').map((alert) => alert.text()).join(" ")).toContain("Protokoll");
});

test("an empty finished collection and a disabled idle action are explicit", async () => {
  const panel = mountPanel();
  await panel.setProps({ stage: "finished", outcome: "completed", counts: { discovered: 0, processed: 0, skipped: 0, failed: 0, unprocessed: 0, warned: 0 } });
  expect(panel.text()).toContain("Keine geeigneten DOCX-Dokumente");
  expect(panel.find("progress").exists()).toBe(false);
  await panel.setProps({ stage: null, disabled: true });
  expect(panel.get("button").attributes("disabled")).toBeDefined();
});

test("busy and protected-review errors explain the action needed without exposing codes", async () => {
  const panel = mountPanel();
  await panel.setProps({ stage: null, error: { code: "operation_busy", retryable: true } });
  expect(panel.get('[role="alert"]').text()).toContain("anderer Vorgang");
  await panel.setProps({ stage: "finished", errors: [{ relative_path: "reviewed.docx", code: "confirmation_required", retryable: false }], error: null });
  expect(panel.get('[role="alert"]').text()).toContain("Prüfungen oder Korrekturen");
});
