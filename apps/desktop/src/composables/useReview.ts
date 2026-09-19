import { computed, onScopeDispose, ref, shallowRef } from "vue";
import type { Decisions, Detection, DocumentKey, EntityType, ReviewStatus, ReviewViewData } from "../lib/contracts";
import { reviewApi, safeError, type ReviewApi } from "../lib/ipc";
import type { SafeError } from "../lib/contracts";

export function useReview(api: ReviewApi = reviewApi) {
  const data = shallowRef<ReviewViewData | null>(null), key = ref<DocumentKey | null>(null);
  const decisions = ref<Decisions>({ dismissed_ids: [], manual: [] });
  const notes = ref(""), status = ref<ReviewStatus>("pending"), acknowledged = ref<string[]>([]);
  const busy = ref(false), error = ref<SafeError | null>(null), message = ref("");
  const history = ref<Decisions[]>([]);
  let request = 0;
  const decisionsChanged = computed(() => !!data.value && JSON.stringify(decisions.value) !== JSON.stringify(data.value.decisions));
  const dirty = computed(() => !!data.value && (decisionsChanged.value || notes.value !== data.value.notes
    || status.value !== data.value.status || JSON.stringify(acknowledged.value) !== JSON.stringify(data.value.acknowledged_warnings)));
  const canApprove = computed(() => !!data.value && !decisionsChanged.value && /[^\p{White_Space}]/u.test(data.value.original_text)
    && !data.value.warnings.includes("empty_document") && data.value.warnings.every(code => acknowledged.value.includes(code)));
  const active = computed(() => [...(data.value?.detections.filter(item => !decisions.value.dismissed_ids.includes(item.id)) ?? []), ...decisions.value.manual]);

  function accept(value: ReviewViewData) {
    data.value = value;
    decisions.value = JSON.parse(JSON.stringify(value.decisions));
    notes.value = value.notes; status.value = value.status; acknowledged.value = [...value.acknowledged_warnings];
  }
  function clear() {
    request++; data.value = null; key.value = null; history.value = [];
    decisions.value = { dismissed_ids: [], manual: [] }; notes.value = "";
    acknowledged.value = []; status.value = "pending"; error.value = null; message.value = ""; busy.value = false;
  }
  onScopeDispose(clear);
  async function open(next: DocumentKey) {
    clear(); key.value = { ...next };
    const token = ++request;
    busy.value = true;
    try {
      const value = await api.open({ ...next });
      if (token !== request) return false;
      accept(value); return true;
    } catch (caught) {
      if (token === request) error.value = safeError(caught);
      return false;
    } finally { if (token === request) busy.value = false; }
  }
  function edit(next: Decisions) {
    if (!data.value || busy.value) return;
    history.value.push(JSON.parse(JSON.stringify(decisions.value)));
    decisions.value = next;
    status.value = data.value.warnings.length ? "needs-rework" : "pending";
    message.value = "Korrektur vorgemerkt. Speichern aktualisiert die Ausgabe.";
  }
  function add(span: { start: number; end: number }, entity_type: EntityType) {
    if (!data.value || !Number.isInteger(span.start) || !Number.isInteger(span.end)
      || span.start < 0 || span.end <= span.start || span.end > Array.from(data.value.original_text).length) return;
    edit({ ...decisions.value, manual: [...decisions.value.manual, {
      ...span, id: crypto.randomUUID(), entity_type, origin: "manual", confidence: null, recognizer: "manual",
    }] });
  }
  function dismiss(id: string) {
    if (!active.value.some(item => item.id === id)) return;
    edit({ dismissed_ids: data.value!.detections.some(item => item.id === id)
      ? [...decisions.value.dismissed_ids, id] : [...decisions.value.dismissed_ids], manual: decisions.value.manual.filter(item => item.id !== id) });
  }
  function changeType(id: string, entity_type: EntityType) {
    const item = active.value.find(item => item.id === id);
    if (!item || item.entity_type === entity_type) return;
    const manual: Detection = { ...item, id: crypto.randomUUID(), entity_type, origin: "manual", confidence: null, recognizer: "manual" };
    edit({ dismissed_ids: item.origin === "automatic" ? [...decisions.value.dismissed_ids, id] : [...decisions.value.dismissed_ids],
      manual: [...decisions.value.manual.filter(item => item.id !== id), manual] });
  }
  function undo() {
    if (busy.value || !history.value.length || !data.value) return;
    decisions.value = history.value.pop()!;
    status.value = data.value.warnings.length ? "needs-rework" : "pending";
    message.value = "Letzte Korrektur zurückgenommen.";
  }
  async function save(nextStatus: ReviewStatus = status.value) {
    if (!data.value || busy.value || (nextStatus === "approved" && !canApprove.value)) return false;
    const current = data.value, token = ++request;
    busy.value = true; error.value = null; message.value = "";
    try {
      const value = await api.save(current.key, {
        expected_output_hash: current.expected_output_hash, decisions: JSON.parse(JSON.stringify(decisions.value)),
        status: nextStatus, notes: notes.value, acknowledged_warnings: [...acknowledged.value],
      });
      if (token !== request) return false;
      accept(value); message.value = "Prüfung gespeichert."; return true;
    } catch (caught) {
      if (token === request) error.value = safeError(caught);
      return false;
    } finally { if (token === request) busy.value = false; }
  }
  return { data, key, decisions, notes, status, acknowledged, busy, error, message, dirty, decisionsChanged, canApprove,
    active, canUndo: computed(() => history.value.length > 0), open, clear, add, dismiss, changeType, undo, save };
}
