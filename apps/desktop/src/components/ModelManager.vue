<script setup lang="ts">
import { OnyxButton, OnyxCard, OnyxHeadline, OnyxInput, OnyxTag } from "sit-onyx";
import { computed, ref } from "vue";
import type { SafeError } from "../lib/contracts";
import { ModelSourceSchema, type CheckedModel, type ManagedModel, type ModelJob, type ModelSource } from "../lib/modelContracts";

const props = defineProps<{ models: ManagedModel[]; checked: CheckedModel | null; job: ModelJob | null; busy: boolean; error: SafeError | null }>();
const emit = defineEmits<{ check: [source: ModelSource]; install: [planId: string]; cancel: []; remove: [name: string] }>();
const url = ref(""), inputError = ref("");
const status = computed(() => ({
  downloading: "Wird heruntergeladen", validating: "Wird geprüft", ready: "Bereit", cancelled: "Abgebrochen",
  failed: "Fehlgeschlagen", removing: "Wird entfernt", removed: "Entfernt",
}[props.job?.stage ?? "downloading"]));
const errorText = computed(() => errorMessage(props.error));
const downloadable = computed(() => props.checked !== null && props.checked.model.state !== "ready"
  && !props.models.some(model => model.name === props.checked!.model.name && model.state === "ready"));
const activeJob = computed(() => props.job !== null && ["downloading", "validating", "removing"].includes(props.job.stage));
const cancellable = computed(() => props.busy && activeJob.value);
function bytes(value: number) {
  const [unit, divisor] = value < 1_000_000 ? ["kilobyte", 1_000] : ["megabyte", 1_000_000];
  return new Intl.NumberFormat("de-DE", { style: "unit", unit, maximumFractionDigits: 1 }).format(value / divisor);
}
function state(model: ManagedModel) { return { available: "Verfügbar", ready: "Bereit", invalid: "Ungültig", removing: "Wird entfernt" }[model.state]; }
function sourceUrl(model: ManagedModel) { return `https://huggingface.co/${model.repository}`; }
function errorMessage(error: SafeError | null) {
  if (!error) return "";
  const messages: Record<string, string> = {
    invalid_model_source: "Die Modelladresse wurde abgelehnt. Prüfen Sie die Hugging-Face-Repository-Adresse.",
    model_network_unsafe: "Die Modellquelle ist nicht sicher. Prüfen Sie die Repository-Adresse.",
    model_network_timeout: "Der Download hat zu lange gedauert. Prüfen Sie die Netzwerkverbindung und versuchen Sie es erneut.",
    model_network_failed: "Der Download ist fehlgeschlagen. Prüfen Sie die Netzwerkverbindung und versuchen Sie es erneut.",
    model_access_denied: "Auf das Modell kann nicht zugegriffen werden. Prüfen Sie die Freigabe auf Hugging Face.",
    model_unavailable: "Das Modell ist nicht verfügbar. Prüfen Sie Quelle und Revision und versuchen Sie es erneut.",
    model_metadata_invalid: "Die Modellmetadaten sind ungültig. Wählen Sie ein unterstütztes Modell.",
    model_artifacts_missing: "Zum Modell fehlen erforderliche Dateien. Prüfen Sie Quelle und Revision.",
    model_artifacts_unsafe: "Die Modelldateien wurden aus Sicherheitsgründen abgelehnt.",
    model_remote_code_unsupported: "Dieses Modell benötigt nicht unterstützten Remote-Code und kann nicht installiert werden.",
    model_labels_invalid: "Die Modell-Labels sind nicht für die Redaktion geeignet.",
    model_incompatible: "Das Modell ist nicht kompatibel. Wählen Sie ein unterstütztes NER-Modell.",
    model_source_mismatch: "Die heruntergeladene Revision passt nicht zur geprüften Quelle. Prüfen Sie das Modell erneut.",
    model_hash_mismatch: "Die Integritätsprüfung der Modelldateien ist fehlgeschlagen. Prüfen Sie das Modell erneut.",
    model_size_mismatch: "Die Größe der Modelldateien stimmt nicht mit der geprüften Quelle überein. Prüfen Sie das Modell erneut.",
    model_not_found: "Das gespeicherte Modell wurde nicht gefunden. Prüfen Sie es erneut.",
    model_path_unsafe: "Der Modellpfad wurde aus Sicherheitsgründen abgelehnt.",
    model_store_busy: "Der Modellspeicher wird gerade verwendet. Warten Sie kurz und versuchen Sie es erneut.",
    model_in_use: "Das Modell wird noch verwendet. Entfernen Sie es erst nach Abschluss der Verwendung.",
    model_insufficient_space: "Für das Modell ist nicht genügend Speicherplatz frei. Geben Sie Speicher frei und versuchen Sie es erneut.",
    model_remove_failed: "Das Modell konnte nicht entfernt werden. Schließen Sie verwendende Vorgänge und versuchen Sie es erneut.",
    model_store_read_only: "Der Modellspeicher ist schreibgeschützt. Verschieben oder kopieren Sie den gesamten App-Ordner in einen beschreibbaren Speicherort und versuchen Sie es erneut.",
    storage_read_only: "Der Modellspeicher ist schreibgeschützt. Verschieben oder kopieren Sie den gesamten App-Ordner in einen beschreibbaren Speicherort und versuchen Sie es erneut.",
    invalid_model_manifest: "Die gespeicherte Modellbeschreibung ist ungültig. Prüfen Sie das Modell erneut.",
    invalid_request: "Die Modellanfrage war ungültig. Prüfen Sie die Eingabe und versuchen Sie es erneut.",
    message_too_large: "Die Modellantwort war zu groß und wurde abgelehnt. Prüfen Sie ein anderes Modell.",
    model_operation_failed: "Das Modell konnte nicht verarbeitet werden. Bitte versuchen Sie es erneut.",
    model_plan_expired: "Die Modellprüfung ist abgelaufen. Prüfen Sie das Modell erneut.",
    invalid_model_protocol: "Die Modellverwaltung antwortet ungültig. Starten Sie die App neu.",
    model_worker_unavailable: "Die Modellverwaltung ist nicht verfügbar. Starten Sie die App neu.",
    model_install_interrupted: "Die Installation wurde unterbrochen. Versuchen Sie es erneut.",
    operation_busy: "Ein anderer Vorgang läuft. Bitte warten Sie, bis er beendet ist.",
    operation_cancelled: "Der Vorgang wurde abgebrochen.",
    state_unavailable: "Der Modellstatus ist nicht verfügbar. Starten Sie die App neu.",
  };
  return messages[error.code] ?? (error.retryable
    ? "Der Vorgang wurde unterbrochen. Bitte versuchen Sie es erneut."
    : "Das Modell konnte nicht verarbeitet werden. Bitte wählen Sie ein anderes unterstütztes Modell.");
}
function retrySource(model: ManagedModel): ModelSource {
  return model.catalog_key ? { kind: "catalog", key: model.catalog_key } : { kind: "receipt", name: model.name };
}
function removalActive(name: string) { return cancellable.value && props.job?.model_name === name; }
function checkUrl() {
  const source = ModelSourceSchema.safeParse({ kind: "url", url: url.value.trim() });
  if (!source.success) { inputError.value = "Geben Sie eine gültige Hugging Face-Repository-Adresse ein."; return; }
  inputError.value = ""; emit("check", source.data);
}
</script>

<template>
  <OnyxCard class="model-manager" role="region" aria-labelledby="models-heading">
    <OnyxHeadline id="models-heading" is="h2">Modelle</OnyxHeadline>
    <p>Modelle werden lokal geprüft und gespeichert. Dokumente verlassen dabei nicht die App.</p>
    <form class="model-import" @submit.prevent="checkUrl">
      <OnyxInput v-model="url" data-testid="model-url" label="Hugging-Face-Repository-Adresse" placeholder="https://huggingface.co/anbieter/modell" :disabled="busy" :error="inputError || undefined" />
      <OnyxButton label="Prüfen" type="submit" :disabled="busy" />
    </form>
    <p v-if="errorText" role="alert">{{ errorText }}</p>
    <section v-if="checked" data-testid="checked-model" aria-live="polite">
      <h3>{{ checked.model.title }}</h3>
      <p>Prüfung abgeschlossen. Das Modell wird vor der Bereitstellung lokal validiert.</p>
      <small>Quelle: <a data-testid="model-source" :href="sourceUrl(checked.model)" target="_blank" rel="noopener noreferrer">{{ checked.model.repository }}</a> · Revision: {{ checked.model.version }}</small>
      <small>Noch herunterzuladen: {{ bytes(checked.model.download_bytes) }} · Labels: {{ checked.model.entity_types.join(', ') }}</small>
      <small>Lizenz: <a v-if="checked.model.license" data-testid="model-license" :href="sourceUrl(checked.model)" target="_blank" rel="noopener noreferrer">{{ checked.model.license }}</a><strong v-else>Nicht angegeben</strong></small>
      <OnyxButton v-if="downloadable" data-testid="download-model" label="Herunterladen" type="button" :disabled="busy" @click="emit('install', checked.plan_id)" />
    </section>
    <section v-if="job && job.stage !== 'ready' && job.stage !== 'removed'" class="model-job" aria-live="polite" role="status">
      <strong>{{ status }}</strong>
      <progress v-if="activeJob && job.total_bytes" :value="job.downloaded_bytes" :max="job.total_bytes">{{ job.downloaded_bytes }} / {{ job.total_bytes }}</progress>
      <span v-if="activeJob">{{ bytes(job.downloaded_bytes) }} von {{ bytes(job.total_bytes) }}</span>
      <OnyxButton v-if="activeJob" data-testid="cancel-job" label="Abbrechen" type="button" mode="outline" :disabled="!cancellable" @click="emit('cancel')" />
    </section>
    <ul class="models">
      <li v-for="model in models" :key="model.name">
        <div>
          <div class="model-title"><strong>{{ model.title }}</strong><OnyxTag :label="state(model)" /></div>
          <small>Quelle: <a :href="sourceUrl(model)" target="_blank" rel="noopener noreferrer">{{ model.repository }}</a> · Revision: {{ model.version }}</small>
          <small>{{ model.state === 'ready' || model.state === 'removing' || (model.state === 'invalid' && model.installed_bytes > 0) ? 'Belegt' : 'Noch herunterzuladen' }}: {{ bytes(model.state === 'ready' || model.state === 'removing' || (model.state === 'invalid' && model.installed_bytes > 0) ? model.installed_bytes : model.download_bytes) }} · Labels: {{ model.entity_types.join(', ') }}</small>
          <small>Lizenz: <a v-if="model.license" :href="sourceUrl(model)" target="_blank" rel="noopener noreferrer">{{ model.license }}</a><strong v-else>Nicht angegeben</strong></small>
          <small v-if="model.error" role="status">{{ errorMessage(model.error) }}</small>
          <small v-if="model.used_by_pairs.length">Wird verwendet von: {{ model.used_by_pairs.map(pair => pair.name).join(', ') }}</small>
          <small v-if="(model.state === 'ready' || model.state === 'invalid') && model.used_by_pairs.length">Entfernen erst möglich, wenn keine gespeicherten Paare dieses Modell verwenden.</small>
          <small v-if="model.state === 'removing'">{{ removalActive(model.name) ? "Die Entfernung wird abgeschlossen. Dieser Eintrag wird danach aktualisiert." : "Die Entfernung wurde unterbrochen. Setzen Sie sie fort." }}</small>
        </div>
        <OnyxButton v-if="model.state === 'available' && model.catalog_key" label="Prüfen" type="button" mode="outline" :disabled="busy" @click="emit('check', { kind: 'catalog', key: model.catalog_key })" />
        <OnyxButton v-else-if="model.state === 'available'" data-testid="check-receipt" label="Erneut prüfen" type="button" mode="outline" :disabled="busy" @click="emit('check', retrySource(model))" />
        <OnyxButton v-if="model.state === 'invalid'" :data-testid="model.catalog_key ? 'retry-catalog' : 'retry-receipt'" label="Erneut prüfen" type="button" mode="outline" :disabled="busy" @click="emit('check', retrySource(model))" />
        <OnyxButton v-else-if="model.state === 'removing' && !removalActive(model.name)" data-testid="resume-removal" label="Entfernen fortsetzen" type="button" color="danger" mode="outline" :disabled="busy || model.used_by_pairs.length > 0" @click="emit('remove', model.name)" />
        <OnyxButton v-if="model.state === 'ready' || (model.state === 'invalid' && model.installed_bytes > 0)" data-testid="remove-model" label="Entfernen" type="button" color="danger" mode="outline" :disabled="busy || model.used_by_pairs.length > 0" @click="emit('remove', model.name)" />
      </li>
    </ul>
  </OnyxCard>
</template>

<style scoped>
.model-manager, .model-import, .models, .models li { display: grid; gap: var(--onyx-spacing-md); }
.model-import { grid-template-columns: minmax(0, 1fr) auto; align-items: end; }
.models { margin: 0; padding: 0; list-style: none; }
.models li { grid-template-columns: minmax(0, 1fr) auto; align-items: center; padding-block: var(--onyx-spacing-md); border-top: 1px solid var(--onyx-color-component-border-neutral); }
.model-title { display: flex; flex-wrap: wrap; gap: var(--onyx-spacing-sm); align-items: center; }
small { display: block; overflow-wrap: anywhere; color: var(--onyx-color-text-icons-neutral-medium); }
.model-job { display: grid; gap: var(--onyx-spacing-sm); }
progress { inline-size: 100%; accent-color: var(--onyx-color-base-primary-400); }
h2, h3, p { margin: 0; }
@media (max-width: 40rem) { .model-import, .models li { grid-template-columns: 1fr; } }
</style>
