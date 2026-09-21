import { computed, ref, type Ref } from "vue";
import type { SafeError, Settings } from "../lib/contracts";
import { pairApi, safeError } from "../lib/ipc";

export interface PairApi {
  listPairs(): Promise<Settings>;
  addPair(name: string, sourceFolder: string, targetFolder: string, createTarget: boolean, modelName: string): Promise<Settings>;
  renamePair(pairId: string, name: string): Promise<Settings>;
  selectPair(pairId: string): Promise<Settings>;
  removePair(pairId: string): Promise<Settings>;
}

export function isSelectedPair(selectedId: string | null, eventPairId: string): boolean {
  return selectedId !== null && selectedId === eventPairId;
}

export function usePairs(initialSettings: Settings, api: PairApi = pairApi) {
  const settings: Ref<Settings> = ref(initialSettings);
  const error = ref<SafeError | null>(null);
  const busy = ref(false);
  let latestRequest = 0;
  const selectedPair = computed(() => settings.value.sync_pairs.find(
    (pair) => pair.id === settings.value.selected_sync_pair_id,
  ) ?? null);

  async function run(request: () => Promise<Settings>): Promise<void> {
    const requestId = ++latestRequest;
    busy.value = true;
    try {
      const next = await request();
      if (requestId === latestRequest) {
        settings.value = next;
        error.value = null;
      }
    } catch (caught) {
      if (requestId === latestRequest) error.value = safeError(caught);
    } finally {
      if (requestId === latestRequest) busy.value = false;
    }
  }

  return {
    settings,
    selectedPair,
    error,
    busy,
    addPair: (name: string, source: string, target: string, createTarget: boolean, modelName: string) => run(() => api.addPair(name, source, target, createTarget, modelName)),
    renamePair: (pairId: string, name: string) => run(() => api.renamePair(pairId, name)),
    selectPair: (pairId: string) => run(() => api.selectPair(pairId)),
    removePair: (pairId: string) => run(() => api.removePair(pairId)),
    applyPairUpdate(pairId: string, apply: () => void) {
      if (isSelectedPair(settings.value.selected_sync_pair_id, pairId)) apply();
    },
  };
}
