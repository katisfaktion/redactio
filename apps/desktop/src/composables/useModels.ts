import { computed, onScopeDispose, ref } from "vue";
import type { ModelInfo, SafeError } from "../lib/contracts";
import { ModelJobSchema, type CheckedModel, type ManagedModel, type ModelJob, type ModelSource } from "../lib/modelContracts";
import { safeError } from "../lib/ipc";
import { modelApi, type ModelApi } from "../lib/modelIpc";

const terminal = new Set<ModelJob["stage"]>(["ready", "cancelled", "failed", "removed"]);

export function useModels(api: ModelApi = modelApi) {
  const models = ref<ManagedModel[]>([]), checked = ref<CheckedModel | null>(null);
  const job = ref<ModelJob | null>(null), busy = ref(false), error = ref<SafeError | null>(null);
  const activeJobId = ref<string | null>(null);
  const readyModels = computed<ModelInfo[]>(() => models.value.filter(model => model.state === "ready").map(model => ({
    name: model.name, version: model.version, compatible: true, entity_types: model.entity_types,
  })));
  let disposed = false, unlisten: (() => void) | null = null;
  let timer: ReturnType<typeof setInterval> | null = null, starting = false, listenerFailed = false;
  const early: ModelJob[] = [];

  function clearTimer() { if (timer) clearInterval(timer); timer = null; }
  function acceptProgress(value: unknown) {
    const parsed = ModelJobSchema.safeParse(value);
    if (!parsed.success) return;
    if (!activeJobId.value) {
      if (starting) { early.push(parsed.data); if (early.length > 32) early.shift(); }
      return;
    }
    if (parsed.data.job_id !== activeJobId.value) return;
    job.value = parsed.data;
    if (terminal.has(parsed.data.stage)) void recover(parsed.data.job_id);
  }
  const subscribed = api.listen(acceptProgress).then(dispose => {
    if (disposed) dispose(); else unlisten = dispose;
  }).catch(caught => { listenerFailed = true; error.value = safeError(caught); });

  async function refresh() {
    try {
      models.value = await api.list();
      error.value = null;
    } catch (caught) { error.value = safeError(caught); }
  }
  async function recover(id: string) {
    try {
      const result = await api.job(id);
      if (!result || result.job_id !== id || activeJobId.value !== id) return;
      job.value = result;
      if (!terminal.has(result.stage)) return;
      clearTimer();
      await refresh();
      if (activeJobId.value === id) {
        if (result.error && !error.value) error.value = result.error;
        activeJobId.value = null; busy.value = false;
      }
    } catch (caught) { if (activeJobId.value === id) error.value = safeError(caught); }
  }
  async function begin(start: () => Promise<string>) {
    if (busy.value) return;
    error.value = null; job.value = null; busy.value = true; starting = true; early.length = 0;
    await subscribed;
    if (disposed || listenerFailed) { busy.value = false; starting = false; return; }
    try {
      const id = await start();
      if (disposed) return;
      activeJobId.value = id; starting = false;
      for (const value of early) acceptProgress(value);
      timer = setInterval(() => void recover(id), 500);
      await recover(id);
    } catch (caught) {
      error.value = safeError(caught); busy.value = false; starting = false; clearTimer();
    }
  }
  async function check(source: ModelSource) {
    if (busy.value) return;
    error.value = null; busy.value = true;
    try { checked.value = await api.check(source); }
    catch (caught) { error.value = safeError(caught); }
    finally { busy.value = false; }
  }
  async function install(planId: string) { await begin(() => api.install(planId)); }
  async function remove(name: string) { await begin(() => api.remove(name)); }
  async function cancel() {
    const id = activeJobId.value;
    if (!id || !busy.value) return;
    try { await api.cancel(id); await recover(id); }
    catch (caught) { error.value = safeError(caught); }
  }
  void refresh();
  onScopeDispose(() => { disposed = true; clearTimer(); unlisten?.(); });
  return { models, readyModels, checked, job, busy, error, refresh, check, install, cancel, remove };
}
