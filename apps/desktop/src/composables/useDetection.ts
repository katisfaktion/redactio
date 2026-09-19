import { onScopeDispose, ref, watch, type Ref } from "vue";
import { ProcessingConfigSchema, type ModelInfo, type ProcessingConfig, type RulePreview, type SafeError, type Settings, type SyncPair } from "../lib/contracts";
import { detectionApi, pairApi, safeError, type DetectionApi } from "../lib/ipc";

export function useDetection(pair: Ref<SyncPair | null>, apply: (settings: Settings) => void, api: DetectionApi = detectionApi) {
  const models = ref<ModelInfo[]>([]), error = ref<SafeError | null>(null);
  const busy = ref(false), saved = ref(false), result = ref<RulePreview | null>(null);
  let request = 0;
  onScopeDispose(() => { request++; result.value = null; });

  async function run(pairId: string, action: () => Promise<Settings | RulePreview>, save = false) {
    const token = ++request;
    busy.value = true; error.value = null; saved.value = false; result.value = null;
    try {
      const value = await action();
      if (request !== token || pair.value?.id !== pairId) return;
      if ("schema_version" in value) apply(value);
      else result.value = value;
      saved.value = save;
    } catch (caught) {
      if (request !== token || pair.value?.id !== pairId) return;
      error.value = safeError(caught);
      if (["storage_cleanup_required", "storage_durability_uncertain", "storage_recovery_required"].includes(error.value.code)) {
        try {
          const actual = await pairApi.listPairs();
          if (request === token && pair.value?.id === pairId) apply(actual);
        } catch { /* Keep the original safe error visible; the next operation reloads disk. */ }
      }
    } finally {
      if (request === token) busy.value = false;
    }
  }

  watch(() => pair.value?.id, async (id) => {
    request++; result.value = null; saved.value = false; error.value = null; busy.value = false;
    if (!id) return;
    await run(id, async () => {
      const available = await api.listModels();
      if (pair.value?.id === id) models.value = available;
      return api.refresh(id);
    });
  }, { immediate: true });

  return { models, busy, error, saved, result,
    save: (id: string, config: ProcessingConfig) => {
      if (busy.value || pair.value?.id !== id) return Promise.resolve();
      return run(id, () => api.save(id, config), true);
    },
    preview: (id: string, config: ProcessingConfig, text: string) => {
      if (busy.value || pair.value?.id !== id) return Promise.resolve();
      const snapshot = ProcessingConfigSchema.parse(config);
      return run(id, async () => ({ pairId: id, config: snapshot, text, detections: await api.preview(id, snapshot, text) }));
    },
  };
}
