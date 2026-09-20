<script setup lang="ts">
import { OnyxButton, OnyxTable } from "sit-onyx";
import { computed, onUnmounted, ref, watch } from "vue";
import type { DocumentKey, SafeError, ScanReport, ScanState } from "../lib/contracts";
import { pairApi, safeError } from "../lib/ipc";

const props = defineProps<{
  pairId: string;
  invalidation?: { sync_pair_id: string; doc_id?: string } | null;
  disabled?: boolean;
  scan?: (pairId: string) => Promise<ScanReport>;
}>();
const emit = defineEmits<{ busy: [value: boolean]; invalidated: []; scanned: [report: ScanReport]; reprocess: [files: { relative_path: string; doc_id: string }[]]; review: [docId: string]; export: [keys: DocumentKey[]] }>();
const selected = ref<string[]>([]);
const selectedFiles = computed(() => report.value?.files.flatMap((file) => file.doc_id && selected.value.includes(file.doc_id)
  ? [{ relative_path: file.relative_path, doc_id: file.doc_id }] : []) ?? []);
const report = ref<ScanReport | null>(null);
const error = ref<SafeError | null>(null);
const busy = ref(false);
let requestId = 0;
let refreshPending = false;

function setBusy(value: boolean) {
  if (busy.value === value) return;
  busy.value = value;
  emit("busy", value);
}

const stateLabels: Record<ScanState, string> = {
  new: "Neu",
  current: "Aktuell",
  stale: "Veraltet",
  "missing-output": "Ausgabe fehlt",
  conflict: "Konflikt",
  "missing-source": "Quelle fehlt",
  "recovery-pending": "Wiederherstellung ausstehend",
};

const reviewLabels = { pending: "Ausstehend", approved: "Freigegeben", rejected: "Abgelehnt", "needs-rework": "Nacharbeit erforderlich" };
function refreshIfReady() {
  if (!refreshPending || props.disabled || busy.value) return;
  refreshPending = false;
  void scan();
}
watch(() => props.invalidation, (change) => {
  if (!change || change.sync_pair_id !== props.pairId
    || (change.doc_id && !report.value?.files.some(file => file.doc_id === change.doc_id))) return;
  requestId += 1;
  refreshPending = true;
  report.value = null; selected.value = []; error.value = null; setBusy(false);
  emit("invalidated");
  refreshIfReady();
});
watch(() => props.disabled, disabled => { if (!disabled) refreshIfReady(); });

watch(() => props.pairId, () => {
  requestId += 1;
  refreshPending = false;
  report.value = null;
  error.value = null;
  setBusy(false);
  selected.value = [];
});
onUnmounted(() => { requestId += 1; refreshPending = false; setBusy(false); });

async function scan() {
  if (props.disabled || busy.value) return;
  const pairId = props.pairId;
  const currentRequest = ++requestId;
  setBusy(true);
  try {
    const result = await (props.scan ?? pairApi.scanPair)(pairId);
    if (currentRequest === requestId && props.pairId === pairId) {
      report.value = result;
      selected.value = [];
      error.value = null;
      emit("scanned", result);
    }
  } catch (caught) {
    if (currentRequest === requestId && props.pairId === pairId) {
      error.value = safeError(caught);
      report.value = null;
    }
  } finally {
    if (currentRequest === requestId && props.pairId === pairId) setBusy(false);
  }
}

function formatMtime(value: string | null): string {
  if (value === null) return "—";
  return new Intl.DateTimeFormat("de-DE", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}
</script>

<template>
  <div class="document-list" aria-live="polite">
    <div class="list-header">
      <OnyxButton
        label="Quellordner einlesen"
        type="button"
        :loading="busy"
        :disabled="busy || disabled"
        @click="scan"
      />
      <p v-if="busy" role="status">Dokumentliste wird aktualisiert …</p>
      <p v-else-if="report">{{ report.files.length }} Dokumente gefunden · {{ report.errors.length }} Lesefehler</p>
    </div>
    <slot />

    <p v-if="error" class="error" role="alert">
      Quellordner konnte nicht eingelesen werden. Bitte prüfen Sie, ob er verfügbar ist.
    </p>

    <template v-if="report">
      <p v-if="report.files.some(file => file.state === 'current' && file.doc_id)" class="review-hint">Bereit zur Prüfung: Öffnen Sie ein verarbeitetes Dokument über „doc-… prüfen“.</p>
      <OnyxTable v-if="report.files.length" striped with-page-scrolling>
        <template #head>
          <tr>
            <th scope="col">Auswahl</th>
            <th scope="col">Name</th>
            <th scope="col">Status</th>
            <th scope="col">Geändert</th>
            <th scope="col">Prüfung</th>
          </tr>
        </template>
        <tr v-for="file in report.files" :key="file.relative_path">
          <td><input v-if="file.doc_id" v-model="selected" type="checkbox" :value="file.doc_id" :aria-label="`${file.relative_path} auswählen`" :disabled="busy || disabled" /></td>
          <td>{{ file.relative_path }}</td>
          <td>{{ stateLabels[file.state] }}</td>
          <td>{{ formatMtime(file.mtime) }}</td>
          <td><div class="review-cell"><span>{{ file.review_status ? reviewLabels[file.review_status] : "—" }}</span> <OnyxButton v-if="file.doc_id" :data-testid="`review-${file.doc_id}`" :label="`${file.doc_id} prüfen`" type="button" mode="outline" :disabled="busy || disabled || file.state !== 'current'" @click="emit('review', file.doc_id)" /></div></td>
        </tr>
      </OnyxTable>
      <div v-if="report.files.some(file => file.doc_id)" class="selection-actions">
        <OnyxButton label="Auswahl erneut verarbeiten" type="button" mode="outline" :disabled="busy || disabled || !selectedFiles.length" @click="emit('reprocess', selectedFiles)" />
        <OnyxButton data-testid="export-selection" label="Auswahl freigegeben exportieren" type="button" mode="outline" :disabled="busy || disabled || !selectedFiles.length" @click="emit('export', selectedFiles.map(file => ({ sync_pair_id: pairId, doc_id: file.doc_id })))" />
      </div>
      <p v-if="!report.files.length && !report.errors.length" class="empty">
        Keine geeigneten DOCX-Dokumente gefunden.
      </p>
      <section v-if="report.errors.length" class="scan-errors" aria-labelledby="scan-errors-heading">
        <h3 id="scan-errors-heading">Nicht lesbare Einträge</h3>
        <ul>
          <li v-for="failure in report.errors" :key="failure.relative_path">
            {{ failure.relative_path }} konnte nicht gelesen werden.
          </li>
        </ul>
      </section>
    </template>
  </div>
</template>

<style scoped>
.document-list { display: grid; gap: var(--onyx-spacing-lg); }
.list-header, .selection-actions, .review-cell { display: flex; flex-wrap: wrap; align-items: center; gap: var(--onyx-spacing-sm) var(--onyx-spacing-lg); }
.list-header p { color: var(--onyx-color-text-icons-neutral-medium); }
.review-cell { justify-content: space-between; }
.review-cell > span { flex: 1 1 8rem; }
.review-hint { padding: var(--onyx-spacing-sm) var(--onyx-spacing-md); border-inline-start: 3px solid var(--onyx-color-text-icons-primary-intense); background: var(--onyx-color-base-background-tinted); }
p { margin: 0; }
td { overflow-wrap: anywhere; }
.error { color: var(--onyx-color-text-icons-danger-intense); }
.empty { color: var(--onyx-color-text-icons-neutral-medium); }
.scan-errors { border-inline-start: var(--onyx-spacing-2xs) solid var(--onyx-color-component-border-danger); padding-inline-start: var(--onyx-spacing-md); }
.scan-errors h3, .scan-errors ul, .error, .empty { margin-block: 0; }
@media (max-width: 650px) { .list-header p { flex-basis: 100%; } }
</style>
