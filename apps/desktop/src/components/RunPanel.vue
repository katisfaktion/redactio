<script setup lang="ts">
import { OnyxButton } from "sit-onyx";
import { computed, useId } from "vue";
import type { SafeError } from "../lib/contracts";

const props = defineProps<{
  pairName: string;
  counts: { discovered: number; processed: number; skipped: number; failed: number; unprocessed: number; warned: number };
  stage: "initializing" | "scanning" | "processing" | "finished" | null;
  outcome?: "completed" | "completed-with-errors" | "cancelled" | "failed";
  cancelling?: boolean;
  disabled?: boolean;
  error?: SafeError | null;
  errors?: (SafeError & { relative_path: string })[];
  auditWarning?: boolean;
}>();
defineEmits<{ start: []; cancel: [] }>();

const headingId = useId();
const active = computed(() => props.stage !== null && props.stage !== "finished");
const completed = computed(() => props.counts.processed + props.counts.skipped + props.counts.failed);
const stageLabels = {
  initializing: "Vorbereitung der Verarbeitung …",
  scanning: "Quellordner wird eingelesen …",
  processing: "Dokumente werden verarbeitet …",
  finished: "Verarbeitung beendet",
};
const outcomeLabels = {
  completed: "Abgeschlossen",
  "completed-with-errors": "Mit Fehlern abgeschlossen",
  cancelled: "Abgebrochen",
  failed: "Verarbeitung fehlgeschlagen",
};
const status = computed(() => {
  if (active.value && props.cancelling) return "Abbruch angefordert. Der laufende Arbeitsschritt wird beendet; bei Zeitüberschreitung wird die Verarbeitung gestoppt.";
  if (props.stage === "finished" && props.outcome) return outcomeLabels[props.outcome];
  return props.stage ? stageLabels[props.stage] : "Bereit zur Verarbeitung";
});
const errorLabels: Record<string, string> = {
  operation_busy: "Ein anderer Vorgang läuft bereits. Bitte warten Sie, bis er beendet ist.",
  file_busy: "Ein anderer Vorgang verwendet dieses Ordnerpaar. Bitte versuchen Sie es danach erneut.",
  confirmation_required: "Gespeicherte Prüfungen oder Korrekturen sind geschützt. Wählen Sie das Dokument aus und bestätigen Sie „Auswahl erneut verarbeiten“.",
  output_conflict: "Die Ausgabe wurde außerhalb der App verändert. Sie wurde nicht überschrieben.",
  review_conflict: "Die gespeicherte Prüfung ist nicht verfügbar oder wurde verändert.",
  recovery_pending: "Eine unterbrochene Verarbeitung muss mit den ursprünglichen Daten wiederhergestellt werden.",
  processing_version_changed: "Die Verarbeitungskomponenten haben sich geändert. Bitte speichern Sie die Erkennungseinstellungen erneut.",
};
</script>

<template>
  <section class="run-panel" :aria-labelledby="headingId">
    <h2 :id="headingId">Verarbeitung: {{ pairName }}</h2>
    <p role="status" aria-live="polite">
      {{ status }}
      <template v-if="stage === 'processing' || stage === 'finished'">
        {{ completed }} von {{ counts.discovered }} Dokumenten abgeschlossen.
      </template>
    </p>
    <progress
      v-if="active || (stage === 'finished' && counts.discovered > 0)"
      aria-label="Verarbeitungsfortschritt"
      :max="counts.discovered || 1"
      :value="stage === 'processing' || stage === 'finished' ? completed : undefined"
    />
    <ul v-if="stage !== null" class="counts">
      <li>Gefunden: {{ counts.discovered }}</li>
      <li>Verarbeitet: {{ counts.processed }}</li>
      <li>Davon mit Warnungen: {{ counts.warned }}</li>
      <li>Übersprungen: {{ counts.skipped }}</li>
      <li>Fehlgeschlagen: {{ counts.failed }}</li>
      <li>Nicht verarbeitet: {{ counts.unprocessed }}</li>
    </ul>
    <p v-if="stage === 'finished' && outcome === 'completed' && counts.discovered === 0">
      Keine geeigneten DOCX-Dokumente gefunden.
    </p>
    <p v-if="error" class="error" role="alert">
      {{ errorLabels[error.code] ?? "Die Verarbeitung konnte nicht durchgeführt werden. Bitte prüfen Sie die Ordner und die lokalen Verarbeitungskomponenten." }}
    </p>
    <div v-if="errors?.length" class="error" role="alert">
      <p>Diese Dokumente konnten nicht verarbeitet werden:</p>
      <ul>
        <li v-for="failure in errors" :key="failure.relative_path">{{ failure.relative_path }} — {{ errorLabels[failure.code] ?? "Das Dokument konnte nicht verarbeitet werden." }}</li>
      </ul>
    </div>
    <p v-if="auditWarning" class="error" role="alert">
      Das Protokoll konnte nicht vollständig gespeichert werden. Bereits gespeicherte Ergebnisse bleiben erhalten.
    </p>
    <OnyxButton
      v-if="active"
      label="Abbrechen"
      type="button"
      :disabled="cancelling"
      @click="$emit('cancel')"
    />
    <OnyxButton
      v-else
      label="Verarbeitung starten"
      type="button"
      :disabled="disabled"
      @click="$emit('start')"
    />
  </section>
</template>

<style scoped>
.run-panel { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: var(--onyx-spacing-sm) var(--onyx-spacing-lg); padding: var(--onyx-spacing-md); background: var(--onyx-color-base-background-tinted); border: 1px solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-md); }
h2, p, ul { margin-block: 0; }
h2 { font-size: var(--onyx-font-size-lg); }
.run-panel > :is(progress, ul, .error) { grid-column: 1 / -1; }
progress { width: 100%; accent-color: var(--onyx-color-text-icons-primary-intense); }
.counts { display: flex; flex-wrap: wrap; gap: var(--onyx-spacing-sm) var(--onyx-spacing-lg); padding: 0; list-style: none; color: var(--onyx-color-text-icons-neutral-medium); }
.error { color: var(--onyx-color-text-icons-danger-intense); }
.run-panel > :last-child { grid-column: 2; grid-row: 1 / 3; align-self: center; justify-self: start; }
@media (max-width: 650px) {
  .run-panel { grid-template-columns: minmax(0, 1fr); }
  .run-panel > :last-child { grid-column: 1; grid-row: auto; }
}
</style>
