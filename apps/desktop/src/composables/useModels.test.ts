import { effectScope } from "vue";
import { expect, test, vi } from "vitest";
import type { ModelApi } from "../lib/modelIpc";
import { checked, job, managed } from "../test/modelFixture";
import { useModels } from "./useModels";

const available = managed({ state: "available", installed_bytes: 0, used_by_pairs: [] });
const plan = "22222222-2222-4222-8222-222222222222";
const current = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const previous = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const downloading = job({ job_id: current, model_name: available.name, downloaded_bytes: 1, total_bytes: 2 });

function api(overrides: Partial<ModelApi> = {}): ModelApi {
  return {
    list: async () => [],
    check: async () => checked({ plan_id: plan, model: available }),
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
  const terminal = job({ ...downloading, stage: "ready", downloaded_bytes: 2 });
  const ready = managed({ state: "ready", installed_bytes: 1024 });
  const scope = effectScope();
  const state = scope.run(() => useModels(api({ job: async () => terminal, list: async () => [ready] })))!;
  try {
    await state.check({ kind: "url", url: "https://huggingface.co/acme/medical-ner" });
    await state.install(state.checked.value!.plan_id);
    expect(state.busy.value).toBe(false);
    expect(state.checked.value).toBeNull();
    expect(state.readyModels.value).toEqual([{
      name: ready.name, version: ready.version, compatible: true, entity_types: ready.entity_types,
    }]);
  } finally { scope.stop(); }
});

test("terminal completion cannot regress through late progress, polling, or cancellation", async () => {
  let receive: (value: unknown) => void = () => { throw new Error("not listening"); };
  let result = null as ReturnType<typeof job> | null;
  let resolvePoll!: (value: ReturnType<typeof job> | null) => void;
  let resolveRefresh!: (value: typeof available[]) => void;
  let deferRefresh = false;
  let firstPoll = true;
  const refresh = new Promise<typeof available[]>(resolve => { resolveRefresh = resolve; });
  const terminal = job({ ...downloading, stage: "cancelled", downloaded_bytes: 2 });
  const scope = effectScope();
  const state = scope.run(() => useModels(api({
    listen: async callback => { receive = callback; return () => {}; },
    job: async () => firstPoll
      ? new Promise(resolve => { firstPoll = false; resolvePoll = resolve; })
      : result,
    cancel: async () => {},
    list: async () => deferRefresh ? refresh : [],
  })))!;
  try {
    const installing = state.install(plan);
    await new Promise(resolve => setTimeout(resolve));
    deferRefresh = true; result = terminal;
    receive(terminal);
    await Promise.resolve();
    receive(downloading);
    const cancelling = state.cancel();
    expect(state.job.value?.stage).toBe("cancelled");
    resolvePoll(downloading);
    resolveRefresh([]);
    await installing;
    await cancelling;
    await Promise.resolve();
    expect(state.job.value?.stage).toBe("cancelled");
    expect(state.busy.value).toBe(false);
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
