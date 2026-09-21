import { onScopeDispose, ref, watch, type Ref } from "vue";
import { RunProgressSchema, RunSummarySchema, type RunProgress, type RunSummary, type SafeError } from "../lib/contracts";
import { runApi, safeError } from "../lib/ipc";

type RunApi = Pick<typeof runApi, "listen" | "start" | "cancel"> & {
  summary(pairId: string, runId: string): Promise<unknown>;
};

export function useRun(pairId: Ref<string | null>, api: RunApi = runApi) {
  const busy = ref(false), cancelling = ref(false);
  const progress = ref<RunProgress | null>(null), summary = ref<RunSummary | null>(null);
  const error = ref<SafeError | null>(null);
  let runId: string | null = null, epoch = 0;
  let unlisten: (() => void) | null = null;
  let timer: ReturnType<typeof setInterval> | null = null;
  let polling = false;

  function cleanup() { unlisten?.(); unlisten = null; if (timer) clearInterval(timer); timer = null; }
  function reset() { epoch++; cleanup(); runId = null; busy.value = false; cancelling.value = false; progress.value = null; summary.value = null; error.value = null; }
  watch(pairId, reset);
  onScopeDispose(reset);

  async function recover(selected: string, id: string, current: number) {
    if (polling) return;
    polling = true;
    try {
      const result = RunSummarySchema.nullable().parse(await api.summary(selected, id));
      if (current !== epoch || pairId.value !== selected || runId !== id || !result) return;
      if (result.sync_pair_id !== selected || result.run_id !== id) return;
      summary.value = result; progress.value = result; busy.value = false; cancelling.value = false;
      error.value = result.error; cleanup();
    } catch (caught) { if (current === epoch) error.value = safeError(caught); }
    finally { polling = false; }
  }

  async function start(paths: string[] | null = null, force: string[] = []) {
    const selected = pairId.value;
    if (!selected || busy.value) return;
    reset(); const current = epoch;
    busy.value = true;
    const early: RunProgress[] = [];
    const receive = (payload: unknown) => {
      const parsed = RunProgressSchema.safeParse(payload);
      if (!parsed.success || current !== epoch || pairId.value !== selected || parsed.data.sync_pair_id !== selected) return;
      if (!runId) { early.push(parsed.data); if (early.length > 32) early.shift(); return; }
      if (parsed.data.run_id !== runId) return;
      progress.value = parsed.data;
      if (parsed.data.stage === "finished") void recover(selected, runId, current);
    };
    try {
      const dispose = await api.listen(receive);
      if (current !== epoch) { dispose(); return; }
      unlisten = dispose;
      const id = await api.start(selected, paths, force);
      if (current !== epoch) return;
      runId = id;
      for (const value of early) receive(value);
      timer = setInterval(() => void recover(selected, id, current), 500);
      await recover(selected, id, current);
    } catch (caught) {
      if (current !== epoch) return;
      error.value = safeError(caught); busy.value = false; cleanup();
    }
  }

  async function cancel() {
    if (!pairId.value || !runId || !busy.value || cancelling.value) return;
    const selected = pairId.value, id = runId, current = epoch;
    cancelling.value = true;
    try { await api.cancel(selected, id); }
    catch (caught) { if (current === epoch) { error.value = safeError(caught); cancelling.value = false; } }
    await recover(selected, id, current);
  }
  return { busy, cancelling, progress, summary, error, start, cancel };
}
