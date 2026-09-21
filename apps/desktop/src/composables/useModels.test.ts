import { effectScope } from "vue";
import { expect, test, vi } from "vitest";
import type { ModelApi } from "../lib/modelIpc";
import { useModels } from "./useModels";

const revision = "0123456789abcdef0123456789abcdef01234567";
const available = {
  name: `hf:acme/medical-ner@${revision}`,
  version: revision,
  repository: "acme/medical-ner",
  title: "Acme medical NER",
  license: "apache-2.0",
  entity_types: ["DATE", "PERSON"],
  window_tokens: 512,
  download_bytes: 1024,
  installed_bytes: 0,
  state: "available" as const,
  catalog_key: null,
  used_by_pairs: [],
  error: null,
};
const plan = "22222222-2222-4222-8222-222222222222";
const current = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const previous = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const downloading = {
  job_id: current,
  model_name: available.name,
  stage: "downloading" as const,
  downloaded_bytes: 1,
  total_bytes: 2,
  error: null,
};

function api(overrides: Partial<ModelApi> = {}): ModelApi {
  return {
    list: async () => [],
    check: async () => ({ plan_id: plan, model: available }),
    install: async () => current,
    cancel: async () => {},
    job: async () => null,
    remove: async () => current,
    listen: async () => () => {},
    ...overrides,
  };
}

test("shows offline catalog models without a selected pair", async () => {
  const scope = effectScope();
  const state = scope.run(() => useModels(api({ list: async () => [available] })))!;
  try {
    await state.refresh();
    expect(state.models.value).toEqual([available]);
    expect(state.readyModels.value).toEqual([]);
  } finally { scope.stop(); }
});

test("inspection does not start installation", async () => {
  const install = vi.fn(async () => current);
  const scope = effectScope();
  const state = scope.run(() => useModels(api({ install })))!;
  try {
    await state.check({ kind: "url", url: "https://huggingface.co/acme/medical-ner" });
    expect(state.checked.value?.plan_id).toBe(plan);
    expect(install).not.toHaveBeenCalled();
  } finally { scope.stop(); }
});

test("ignores progress belonging to a previous job", async () => {
  let receive: (value: unknown) => void = () => { throw new Error("not listening"); };
  const scope = effectScope();
  const state = scope.run(() => useModels(api({ listen: async callback => {
    receive = callback;
    return () => {};
  } })))!;
  try {
    await state.install(plan);
    receive({ ...downloading, job_id: previous });
    expect(state.job.value?.job_id).not.toBe(previous);
    receive(downloading);
    expect(state.job.value?.job_id).toBe(current);
  } finally { scope.stop(); }
});

test("terminal job refreshes the registry before exposing a ready model", async () => {
  const terminal = { ...downloading, stage: "ready" as const, downloaded_bytes: 2 };
  const ready = { ...available, state: "ready" as const, installed_bytes: 1024 };
  const scope = effectScope();
  const state = scope.run(() => useModels(api({ job: async () => terminal, list: async () => [ready] })))!;
  try {
    await state.install(plan);
    expect(state.busy.value).toBe(false);
    expect(state.readyModels.value).toEqual([{
      name: ready.name, version: ready.version, compatible: true, entity_types: ready.entity_types,
    }]);
  } finally { scope.stop(); }
});

test("failed refresh keeps the last registry data", async () => {
  let fail = false;
  const scope = effectScope();
  const state = scope.run(() => useModels(api({ list: async () => {
    if (fail) throw { code: "storage_read_only", retryable: false };
    return [available];
  } })))!;
  try {
    await state.refresh();
    fail = true;
    await state.refresh();
    expect(state.models.value).toEqual([available]);
    expect(state.error.value?.code).toBe("storage_read_only");
  } finally { scope.stop(); }
});

test("unmount cleans the model-progress listener", async () => {
  const unlisten = vi.fn();
  const scope = effectScope();
  scope.run(() => useModels(api({ listen: async () => unlisten })));
  await Promise.resolve();
  scope.stop();
  expect(unlisten).toHaveBeenCalledOnce();
});
