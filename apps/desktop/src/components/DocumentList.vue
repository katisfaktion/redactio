<script setup lang="ts">
import { OnyxButton, OnyxTable } from "sit-onyx";
import { ref, watch } from "vue";
import type { SafeError, ScanReport, ScanState } from "../lib/contracts";
import { pairApi, safeError } from "../lib/ipc";

const props = defineProps<{
  pairId: string;
  scan?: (pairId: string) => Promise<ScanReport>;
}>();
const report = ref<ScanReport | null>(null);
const error = ref<SafeError | null>(null);
const busy = ref(false);
let requestId = 0;

const stateLabels: Record<ScanState, string> = {
  new: "Neu",
  current: "Aktuell",
  stale: "Veraltet",
  "missing-output": "Ausgabe fehlt",
  conflict: "Konflikt",
  "missing-source": "Quelle fehlt",
  "recovery-pending": "Wiederherstellung ausstehend",
};

watch(() => props.pairId, () => {
  requestId += 1;
  report.value = null;
  error.value = null;
  busy.value = false;
});

async function scan() {
  const pairId = props.pairId;
  const currentRequest = ++requestId;
  busy.value = true;
  try {
    const result = await (props.scan ?? pairApi.scanPair)(pairId);
    if (currentRequest === requestId && props.pairId === pairId) {
      report.value = result;
      error.value = null;
    }
  } catch (caught) {
    if (currentRequest === requestId && props.pairId === pairId) {
      error.value = safeError(caught);
      report.value = null;
    }
  } finally {
    if (currentRequest === requestId && props.pairId === pairId) busy.value = false;
  }
}

function formatMtime(value: string): string {
  return new Intl.DateTimeFormat("de-DE", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}
</script>

<template>
  <div class="document-list" aria-live="polite">
    <OnyxButton
      label="Quellordner einlesen"
      type="button"
      :loading="busy"
      :disabled="busy"
      @click="scan"
    />

    <p v-if="error" class="error" role="alert">
      Quellordner konnte nicht eingelesen werden. Bitte prüfen Sie, ob er verfügbar ist.
    </p>

    <template v-if="report">
      <OnyxTable v-if="report.files.length" striped with-page-scrolling>
        <template #head>
          <tr>
            <th scope="col">Name</th>
            <th scope="col">Status</th>
            <th scope="col">Geändert</th>
          </tr>
        </template>
        <tr v-for="file in report.files" :key="file.relative_path">
          <td>{{ file.relative_path }}</td>
          <td>{{ stateLabels[file.state] }}</td>
          <td>{{ formatMtime(file.mtime) }}</td>
        </tr>
      </OnyxTable>
      <p v-else-if="!report.errors.length" class="empty">
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
.document-list > :first-child { justify-self: start; }
.error { color: var(--onyx-color-text-icons-danger-intense); }
.empty { color: var(--onyx-color-text-icons-neutral-medium); }
.scan-errors { border-inline-start: var(--onyx-spacing-2xs) solid var(--onyx-color-border-danger); padding-inline-start: var(--onyx-spacing-md); }
.scan-errors h3, .scan-errors ul, .error, .empty { margin-block: 0; }
</style>
