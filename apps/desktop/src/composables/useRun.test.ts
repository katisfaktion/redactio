import { effectScope, ref } from "vue";
import { flushPromises } from "@vue/test-utils";
import { expect, test, vi } from "vitest";
import { useRun } from "./useRun";

const pair = "11111111-1111-4111-8111-111111111111";
const other = "22222222-2222-4222-8222-222222222222";
const runId = "33333333-3333-4333-8333-333333333333";
const progress = { sync_pair_id: pair, run_id: runId, stage: "processing", discovered: 2, processed: 1, skipped: 0, failed: 0, unprocessed: 1, warned: 0 };
const summary = { ...progress, stage: "finished", outcome: "cancelled", errors: [], error: null, audit_warning: true };

test("subscribes before start, binds early progress, filters identities, and cleans up", async () => {
  let listener!: (payload: unknown) => void;
  const order: string[] = []; const unlisten = vi.fn();
  const api = {
    listen: async (callback: (value: unknown) => void) => { order.push("listen"); listener = callback; return unlisten; },
    start: async () => { order.push("start"); listener(progress); return runId; },
    cancel: async () => {}, summary: async () => null,
  };
  const scope = effectScope(); const selected = ref<string | null>(pair);
  const run = scope.run(() => useRun(selected, api))!;
  await run.start(); expect(order).toEqual(["listen", "start"]);
  expect(run.progress.value?.processed).toBe(1);
  listener({ ...progress, sync_pair_id: other, processed: 2, unprocessed: 0 });
  listener({ ...progress, run_id: other, processed: 2, unprocessed: 0 });
  listener({ ...progress, processed: 3 });
  expect(run.progress.value?.processed).toBe(1);
  selected.value = other; await flushPromises();
  expect(run.progress.value).toBeNull(); expect(unlisten).toHaveBeenCalledTimes(1);
  scope.stop();
});

test("authoritative polling recovers a missed finish and releases busy state", async () => {
  vi.useFakeTimers();
  const scope = effectScope(); let final = false; const unlisten = vi.fn();
  const run = scope.run(() => useRun(ref(pair), {
    listen: async () => unlisten, start: async () => runId, cancel: async () => {},
    summary: async () => final ? summary : null,
  }))!;
  await run.start(); expect(run.busy.value).toBe(true);
  final = true; await vi.advanceTimersByTimeAsync(1000);
  expect(run.busy.value).toBe(false); expect(run.summary.value?.audit_warning).toBe(true);
  expect(unlisten).toHaveBeenCalledTimes(1);
  scope.stop(); vi.useRealTimers();
});

test("startup rejection and cancellation errors stay visible without duplicate starts", async () => {
  const scope = effectScope(); let starts = 0; let fail = true;
  const run = scope.run(() => useRun(ref(pair), {
    listen: async () => () => {},
    start: async () => { starts++; if (fail) throw { code: "operation_busy", retryable: true }; return runId; },
    cancel: async () => { throw { code: "unknown_run", retryable: false }; }, summary: async () => null,
  }))!;
  await run.start(); expect(run.busy.value).toBe(false); expect(run.error.value?.code).toBe("operation_busy");
  fail = false; const first = run.start(); await run.start(); await first;
  expect(starts).toBe(2);
  await run.cancel(); expect(run.cancelling.value).toBe(false); expect(run.error.value?.code).toBe("unknown_run");
  scope.stop();
});

test("unmount during listener registration disposes the late listener without starting", async () => {
  let resolve!: (unsubscribe: () => void) => void; const start = vi.fn(async () => runId); const unlisten = vi.fn();
  const scope = effectScope();
  const run = scope.run(() => useRun(ref(pair), {
    listen: () => new Promise<() => void>((done) => { resolve = done; }), start,
    cancel: async () => {}, summary: async () => null,
  }))!;
  const pending = run.start(); scope.stop(); resolve(unlisten); await pending;
  expect(unlisten).toHaveBeenCalledOnce(); expect(start).not.toHaveBeenCalled();
});
