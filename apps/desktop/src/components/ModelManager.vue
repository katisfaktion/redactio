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
const errorText = computed(() => inputError.value || (props.error ? ({
  storage_read_only: "Der Modellspeicher ist schreibgeschützt. Wählen Sie einen beschreibbaren Speicherort und versuchen Sie es erneut.",
  invalid_model_source: "Die Modelladresse wurde abgelehnt. Prüfen Sie die Hugging-Face-Repository-Adresse.",
}[props.error.code] ?? "Das Modell konnte nicht verarbeitet werden. Bitte versuchen Sie es erneut.") : ""));
function bytes(value: number) { return new Intl.NumberFormat("de-DE", { style: "unit", unit: "megabyte", maximumFractionDigits: 1 }).format(value / 1_000_000); }
function state(model: ManagedModel) { return { available: "Verfügbar", ready: "Bereit", invalid: "Ungültig", removing: "Wird entfernt" }[model.state]; }
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
      <OnyxInput v-model="url" data-testid="model-url" label="Hugging-Face-Repository-Adresse" placeholder="https://huggingface.co/anbieter/modell" :disabled="busy" />
      <OnyxButton label="Prüfen" type="submit" :disabled="busy" />
    </form>
    <p v-if="errorText" role="alert">{{ errorText }}</p>
    <section v-if="checked" data-testid="checked-model" aria-live="polite">
      <h3>{{ checked.model.title }}</h3>
      <p>Prüfung abgeschlossen. Das Modell wird vor der Bereitstellung lokal validiert.</p>
      <OnyxButton v-if="checked.model.state !== 'ready'" data-testid="download-model" label="Herunterladen" type="button" :disabled="busy" @click="emit('install', checked.plan_id)" />
    </section>
    <section v-if="job" class="model-job" aria-live="polite" role="status">
      <strong>{{ status }}</strong>
      <progress v-if="job.total_bytes" :value="job.downloaded_bytes" :max="job.total_bytes">{{ job.downloaded_bytes }} / {{ job.total_bytes }}</progress>
      <span>{{ bytes(job.downloaded_bytes) }} von {{ bytes(job.total_bytes) }}</span>
      <OnyxButton label="Abbrechen" type="button" mode="outline" :disabled="!busy" @click="emit('cancel')" />
    </section>
    <ul class="models">
      <li v-for="model in models" :key="model.name">
        <div>
          <div class="model-title"><strong>{{ model.title }}</strong><OnyxTag :label="state(model)" /></div>
          <small>Quelle: {{ model.repository }} · Revision: {{ model.version }}</small>
          <small>Größe: {{ bytes(model.state === 'ready' ? model.installed_bytes : model.download_bytes) }} · Labels: {{ model.entity_types.join(', ') }}</small>
          <small>Lizenz: <strong>{{ model.license ?? "Nicht angegeben" }}</strong></small>
          <small v-if="model.used_by_pairs.length">Wird verwendet von: {{ model.used_by_pairs.map(pair => pair.name).join(', ') }}</small>
        </div>
        <OnyxButton v-if="model.state === 'available' && model.catalog_key" label="Prüfen" type="button" mode="outline" :disabled="busy" @click="emit('check', { kind: 'catalog', key: model.catalog_key })" />
        <OnyxButton v-else-if="model.state === 'ready'" data-testid="remove-model" label="Entfernen" type="button" color="danger" mode="outline" :disabled="busy" @click="emit('remove', model.name)" />
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
