<script setup lang="ts">
import { open } from "@tauri-apps/plugin-dialog";
import { OnyxButton } from "sit-onyx";
import { onBeforeUnmount, onMounted, ref, useId } from "vue";
import type { DocumentKey, ExportSummary, SafeError } from "../lib/contracts";
import { exportApi, safeError } from "../lib/ipc";

const props = defineProps<{ pairId: string; pairName: string; keys: DocumentKey[] }>();
const emit = defineEmits<{ close: []; busy: [value: boolean] }>();
// This dialog owns one immutable selection, including the pair of every document.
const pairId = props.pairId;
const pairName = props.pairName;
const selection = props.keys.map((key) => ({ ...key }));
const dialog = ref<HTMLDialogElement>();
const heading = useId();
const busy = ref(false);
const attempted = ref(false);
const result = ref<ExportSummary | null>(null);
const error = ref<SafeError | null>(null);
let mounted = true;
onMounted(() => dialog.value?.showModal());
onBeforeUnmount(() => { mounted = false; dialog.value?.close(); });

const messages: Record<string, string> = {
  approval_required: "Das Dokument ist nicht freigegeben.",
  reprocess_required: "Quelle oder Verarbeitungseinstellungen haben sich geändert. Bitte erneut verarbeiten und prüfen.",
  output_conflict: "Die Arbeitsausgabe wurde verändert. Bitte erneut prüfen.",
  review_mismatch: "Die gespeicherte Prüfung stimmt nicht mit den aktuellen Dateien überein.",
  invalid_review: "Die gespeicherte Prüfung ist ungültig; leere Dokumente können nicht freigegeben werden.",
  recovery_pending: "Die unterbrochene Verarbeitung muss zuerst wiederhergestellt werden.",
  folder_overlap: "Der Exportordner darf keine Quelle, Arbeitsausgabe oder App-Einstellungen enthalten oder darin liegen.",
  export_not_empty: "Der Exportordner ist nicht leer. Bitte wählen Sie für einen neuen Export einen anderen leeren Ordner.",
  path_exists: "Eine Datei wurde zwischenzeitlich angelegt und nicht überschrieben.",
  path_changed: "Der Ordner oder eine Datei wurde während des Exports verändert. Vorhandene Dateien bleiben erhalten.",
  permission_denied: "Der Zugriff wurde verweigert. Bitte prüfen Sie die Ordnerberechtigungen.",
  file_busy: "Eine Datei oder dieses Ordnerpaar wird bereits verwendet.",
  operation_busy: "Ein anderer Vorgang läuft bereits. Bitte warten Sie auf dessen Abschluss.",
  invalid_export_selection: "Wählen Sie jedes Dokument nur einmal und aus genau einem Ordnerpaar aus.",
  unknown_document: "Mindestens ein Dokument gehört nicht zu diesem Ordnerpaar. Es wurde nichts exportiert.",
  storage_durability_uncertain: "Die Datei kann bereits vorhanden sein; die dauerhafte Speicherung ist nicht bestätigt. Bitte prüfen Sie den Exportordner.",
};
function message(value: SafeError) {
  return messages[value.code] ?? "Der Export konnte nicht bestätigt werden. Eine Datei kann bereits vorhanden sein. Bitte prüfen Sie den Exportordner.";
}
async function run(choose: boolean) {
  if (busy.value || attempted.value) return;
  busy.value = true;
  emit("busy", true);
  try {
    const destination = choose ? await open({ directory: true, multiple: false, title: "Leeren Exportordner auswählen" }) : null;
    if (Array.isArray(destination)) throw { code: "invalid_path", retryable: false };
    const summary = await exportApi.approved(pairId, selection, destination);
    if (mounted) result.value = summary;
  } catch (failure) {
    if (mounted) error.value = safeError(failure);
  } finally {
    if (mounted) {
      attempted.value = true;
      busy.value = false;
      emit("busy", false);
    }
  }
}
async function close() {
  if (busy.value) return;
  if (!attempted.value) {
    await run(false);
    // Preserve cancellation audit warnings until the user has seen them.
    if (error.value || result.value?.audit_warning) return;
  }
  emit("close");
}
</script>

<template>
  <dialog ref="dialog" :aria-labelledby="heading" @cancel.prevent="close">
    <div class="export-content" :aria-busy="busy">
      <h2 :id="heading">Freigegebene Dokumente exportieren: {{ pairName }}</h2>
      <p>Auswahl: {{ selection.map((key) => key.doc_id).join(", ") }}</p>
      <p v-if="!attempted">Der Export benötigt einen leeren Ordner. Nur aktuell freigegebene Markdown-Dateien werden kopiert. Spätere Änderungen können bereits exportierte Dateien nicht zurückrufen.</p>
      <p v-if="busy" role="status">Export wird vorbereitet und geprüft …</p>
      <template v-if="result">
        <p role="status">
          {{ result.cancelled ? "Export abgebrochen." : result.error || result.failed.length ? "Export unvollständig." : "Export abgeschlossen." }}
        </p>
        <div v-if="result.exported.length" data-testid="exported">
          <h3>Exportiert</h3>
          <ul><li v-for="id in result.exported" :key="id">{{ id }}</li></ul>
        </div>
        <div v-if="result.failed.length" data-testid="failed" role="alert">
          <h3>Blockiert oder fehlgeschlagen</h3>
          <ul><li v-for="failure in result.failed" :key="failure.doc_id">{{ failure.doc_id }} — {{ message(failure.error) }}</li></ul>
        </div>
        <p v-if="result.error" role="alert">{{ message(result.error) }}</p>
        <p v-if="result.error || result.failed.length">Bereits exportierte Dateien bleiben erhalten. Prüfen Sie das Ergebnis und verwenden Sie für einen weiteren Export einen neuen leeren Ordner.</p>
        <p v-if="result.audit_warning" role="alert">Das Protokoll konnte nicht vollständig gespeichert werden. Bereits exportierte Dateien bleiben erhalten.</p>
      </template>
      <p v-if="error" role="alert">{{ message(error) }}</p>
      <div class="actions">
        <OnyxButton v-if="!attempted" data-testid="export-start" label="Exportordner auswählen und exportieren" type="button" :disabled="busy || !selection.length" @click="run(true)" />
        <OnyxButton data-testid="export-close" :label="attempted ? 'Schließen' : 'Abbrechen'" type="button" :disabled="busy" @click="close" />
      </div>
    </div>
  </dialog>
</template>

<style scoped>
dialog { max-width: min(48rem, calc(100vw - 2rem)); max-height: calc(100vh - 2rem); border: 1px solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-md); background: var(--onyx-color-base-background-blank); color: var(--onyx-color-text-icons-neutral-intense); }
dialog::backdrop { background: #0008; }
.export-content { display: grid; gap: var(--onyx-spacing-md); padding: var(--onyx-spacing-md); overflow-wrap: anywhere; }
h2, h3, p, ul { margin-block: 0; }
.actions { display: flex; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
[role="alert"] { color: var(--onyx-color-text-icons-danger-intense); }
</style>
