<script setup lang="ts">
import { OnyxButton, OnyxCheckbox, OnyxSelect, OnyxTextarea } from "sit-onyx";
import { computed, nextTick, ref, watch } from "vue";
import type { useReview } from "../composables/useReview";
import { EntityTypeSchema, type EntityType } from "../lib/contracts";
import { codePointOffset, selectionOffsets } from "../lib/selection";

const props = defineProps<{ review: ReturnType<typeof useReview>; pairName: string }>();
const emit = defineEmits<{ back: [] }>();
const entity = ref<EntityType>("PERSON"), keyboard = ref(false), page = ref(0);
const selected = ref<{ start: number; end: number } | null>(null);
const original = ref<HTMLElement>(), textarea = ref<HTMLTextAreaElement>();
const labels: Record<EntityType, string> = {
  PERSON: "Person", LOCATION: "Ort", EMAIL_ADDRESS: "E-Mail-Adresse", PHONE_NUMBER: "Telefonnummer",
  IBAN_CODE: "IBAN", IP_ADDRESS: "IP-Adresse", URL: "Internetadresse", DATE_TIME: "Datum / Uhrzeit", CUSTOM: "Benutzerdefiniert",
};
const entities = EntityTypeSchema.options.map(value => ({ value, label: labels[value] }));
const statuses = { pending: "Ausstehend", approved: "Freigegeben", rejected: "Abgelehnt", "needs-rework": "Nacharbeit erforderlich" };
const errors: Record<string, string> = {
  review_conflict: "Das gespeicherte Ergebnis wurde geändert. Ihre Änderungen bleiben erhalten. Verwerfen Sie sie nur, wenn Sie den aktuellen Stand neu öffnen möchten.",
  output_conflict: "Die Ausgabedatei wurde geändert. Ihre Änderungen bleiben erhalten; prüfen Sie den Konflikt in der Dokumentliste.",
  reprocess_required: "Quelle oder Erkennung hat sich geändert. Das Dokument muss erneut verarbeitet werden.",
  operation_busy: "Eine andere Verarbeitung läuft. Versuchen Sie es danach erneut.",
};
const warnings: Record<string, string> = {
  empty_document: "Keine extrahierbaren Inhalte. Eine Freigabe ist nicht möglich.",
  headers_footers: "Kopf- oder Fußzeilen wurden nicht übernommen.",
};
const visibleDetections = computed(() => props.review.active.value.slice(page.value * 20, (page.value + 1) * 20));
watch(() => props.review.active.value.length, () => { page.value = Math.min(page.value, Math.max(0, Math.ceil(props.review.active.value.length / 20) - 1)); });
watch(() => props.review.data.value, () => { selected.value = null; });

function segments(text: string, spans: { start: number; end: number }[]) {
  const points = Array.from(text), result: { text: string; marked: boolean }[] = [];
  const intervals: { start: number; end: number }[] = [];
  for (const span of [...spans].sort((a, b) => a.start - b.start || a.end - b.end)) {
    const previous = intervals.at(-1);
    if (previous && span.start < previous.end) previous.end = Math.max(previous.end, span.end);
    else intervals.push({ ...span });
  }
  let cursor = 0;
  for (const { start, end } of intervals) {
    if (start > cursor) result.push({ text: points.slice(cursor, start).join(""), marked: false });
    result.push({ text: points.slice(start, end).join(""), marked: true }); cursor = end;
  }
  if (cursor < points.length) result.push({ text: points.slice(cursor).join(""), marked: false });
  return result;
}
const originalSegments = computed(() => keyboard.value ? [] : segments(props.review.data.value?.original_text ?? "", props.review.active.value));
const outputSegments = computed(() => segments(props.review.data.value?.body ?? "", props.review.data.value?.redactions.map(span => ({ start: span.start_offset, end: span.end_offset })) ?? []));
async function keyboardSelection() {
  keyboard.value = !keyboard.value; selected.value = null;
  await nextTick();
  if (keyboard.value) textarea.value?.focus();
  else original.value?.focus();
}
function capture() {
  selected.value = null;
  if (keyboard.value && textarea.value) {
    const control = textarea.value;
    if (control.selectionStart === control.selectionEnd) return;
    try { selected.value = { start: codePointOffset(control.value, control.selectionStart), end: codePointOffset(control.value, control.selectionEnd) }; }
    catch { /* A partial surrogate pair is not a selectable redaction. */ }
  } else {
    const selection = window.getSelection();
    if (original.value && selection) selected.value = selectionOffsets(original.value, selection);
  }
}
function add() {
  if (!selected.value) return;
  props.review.add(selected.value, entity.value); selected.value = null;
}
function restoreSource(event: Event) {
  (event.target as HTMLTextAreaElement).value = props.review.data.value?.original_text ?? "";
  selected.value = null;
}
</script>

<template>
  <section class="review" aria-labelledby="review-heading">
    <div class="actions">
      <h2 id="review-heading">Prüfung: {{ pairName }} · {{ review.key.value?.doc_id }}</h2>
      <OnyxButton label="Zur Dokumentliste" type="button" mode="outline" :disabled="review.busy.value" @click="emit('back')" />
    </div>
    <p v-if="review.busy.value" role="status">Prüfung wird geladen oder gespeichert …</p>
    <p v-if="review.error.value" role="alert">{{ errors[review.error.value.code] ?? "Die Prüfung konnte nicht geladen oder gespeichert werden. Ihre ungespeicherten Änderungen bleiben erhalten." }}</p>
    <p aria-live="polite">{{ review.message.value }}</p>
    <template v-if="review.data.value">
      <p><strong>Status: {{ statuses[review.status.value] }}</strong> · {{ review.dirty.value ? 'Ungespeicherte Änderungen' : 'Gespeicherter Stand' }}</p>
      <section v-if="review.data.value.warnings.length" class="warnings" aria-label="Hinweise zur Extraktion">
        <h3>Inhalte möglicherweise unvollständig</h3>
        <OnyxCheckbox v-for="warning in review.data.value.warnings" :key="warning" :value="warning"
          :label="`${warnings[warning] ?? `Nicht übernommener Inhalt (${warning})`} Hinweis geprüft.`"
          :model-value="review.acknowledged.value.includes(warning)" :disabled="review.busy.value"
          @update:model-value="checked => review.acknowledged.value = checked ? [...review.acknowledged.value, warning] : review.acknowledged.value.filter(code => code !== warning)" />
      </section>
      <div class="actions">
        <OnyxButton data-testid="keyboard-select" :label="keyboard ? 'Markierungen anzeigen' : 'Per Tastatur auswählen'" type="button" mode="outline" :disabled="review.busy.value" @click="keyboardSelection" />
        <OnyxSelect v-model="entity" label="Typ der neuen Schwärzung" list-label="Entitätstypen" :options="entities" :hide-clear-icon="true" :disabled="review.busy.value" />
        <OnyxButton data-testid="add-redaction" label="Auswahl schwärzen" type="button" :disabled="review.busy.value || !selected" @click="add" />
        <OnyxButton data-testid="undo-review" label="Korrektur zurücknehmen" type="button" mode="outline" :disabled="review.busy.value || !review.canUndo.value" @click="review.undo" />
      </div>
      <p v-if="selected" role="status">Auswahl: Position {{ selected.start }}–{{ selected.end }}</p>
      <p v-if="keyboard" id="selection-help">Wählen Sie Text mit Umschalt- und Pfeiltasten aus. Wechseln Sie mit Umschalt+Tab zu „Auswahl schwärzen“. Der Originaltext ist schreibgeschützt.</p>
      <div class="comparison">
        <section aria-label="Originaltext">
          <h3>Originaltext</h3>
          <!-- WebKit's readonly textarea blocks caret navigation; prevent edits while retaining native selection. -->
          <textarea v-if="keyboard" ref="textarea" data-testid="selection-text" class="document-text" aria-readonly="true" spellcheck="false" autocomplete="off" aria-label="Originaltext auswählen" aria-describedby="selection-help" :value="review.data.value.original_text" @beforeinput.prevent @paste.prevent @drop.prevent @input="restoreSource" @select="capture" @keyup="capture" @mouseup="capture" />
          <div v-else ref="original" data-testid="review-original" class="document-text" tabindex="0" aria-label="Originaltext mit Markierungen" @mouseup="capture" @keyup="capture"><template v-for="(part, index) in originalSegments" :key="index"><mark v-if="part.marked">{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></div>
        </section>
        <section aria-label="Ausgabe">
          <h3>Ausgabe</h3>
          <p v-if="review.decisionsChanged.value">Ausgabe des letzten gespeicherten Stands. Speichern aktualisiert die Schwärzungen.</p>
          <div data-testid="review-output" class="document-text" tabindex="0"><template v-for="(part, index) in outputSegments" :key="index"><mark v-if="part.marked">{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></div>
        </section>
      </div>
      <details>
        <summary>Schwärzungen bearbeiten ({{ review.active.value.length }})</summary>
        <p>Konfidenz ist eine Einschätzung des Detektors und keine Datenschutzgarantie.</p>
        <ul class="detections">
          <li v-for="item in visibleDetections" :key="item.id">
            <span>Position {{ item.start }}–{{ item.end }} · {{ item.origin === 'manual' ? 'Manuell' : `Konfidenz ${item.confidence === null ? 'unbekannt' : Math.round(item.confidence * 100) + ' %'}` }}</span>
            <OnyxSelect :model-value="item.entity_type" :label="`Typ an Position ${item.start}–${item.end}`" list-label="Entitätstypen" :options="entities" :hide-clear-icon="true" :disabled="review.busy.value" @update:model-value="value => value && review.changeType(item.id, value as EntityType)" />
            <OnyxButton :label="`Schwärzung ${item.start}–${item.end} entfernen`" type="button" mode="outline" :disabled="review.busy.value" @click="review.dismiss(item.id)" />
          </li>
        </ul>
        <div v-if="review.active.value.length > 20" class="actions">
          <OnyxButton label="Vorherige Schwärzungen" type="button" mode="outline" :disabled="page === 0" @click="page--" />
          <span>Seite {{ page + 1 }} von {{ Math.ceil(review.active.value.length / 20) }}</span>
          <OnyxButton label="Weitere Schwärzungen" type="button" mode="outline" :disabled="(page + 1) * 20 >= review.active.value.length" @click="page++" />
        </div>
      </details>
      <OnyxTextarea v-model="review.notes.value" label="Private Prüfnotizen" :disabled="review.busy.value" />
      <div class="actions">
        <OnyxButton data-testid="save-review" label="Prüfung speichern" type="button" :disabled="review.busy.value || !review.dirty.value" @click="review.save()" />
        <OnyxButton data-testid="approve-review" label="Freigeben" type="button" :disabled="review.busy.value || !review.canApprove.value" @click="review.save('approved')" />
        <OnyxButton label="Ablehnen" type="button" mode="outline" :disabled="review.busy.value" @click="review.save('rejected')" />
        <OnyxButton label="Nacharbeit erforderlich" type="button" mode="outline" :disabled="review.busy.value" @click="review.save('needs-rework')" />
        <OnyxButton label="Als ausstehend speichern" type="button" mode="outline" :disabled="review.busy.value" @click="review.save('pending')" />
      </div>
      <p v-if="review.decisionsChanged.value">Korrekturen zunächst speichern, danach die aktualisierte Ausgabe prüfen und ausdrücklich freigeben.</p>
    </template>
  </section>
</template>

<style scoped>
.review, .warnings, .detections { display: grid; gap: var(--onyx-spacing-md); min-width: 0; }
.actions, .detections li { display: flex; align-items: center; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
.actions h2 { flex: 1 1 18rem; }
.actions :deep(.onyx-select) { max-width: 18rem; }
.comparison { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--onyx-spacing-md); }
.comparison section { min-width: 0; }
.document-text { display: block; white-space: pre-wrap; overflow-wrap: anywhere; width: 100%; height: 22rem; overflow: auto; padding: var(--onyx-spacing-sm); border: 1px solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-sm); background: var(--onyx-color-base-background-blank); color: inherit; font: inherit; box-sizing: border-box; }
textarea.document-text { resize: vertical; }
.document-text:focus-visible, input:focus-visible, summary:focus-visible { outline: 3px solid var(--onyx-color-text-icons-primary-intense); outline-offset: 2px; }
mark { color: inherit; background: var(--onyx-color-base-warning-200, #ffe59b); text-decoration: underline; }
.warnings { border-inline-start: 4px solid #a66b00; padding: var(--onyx-spacing-sm); }
summary { cursor: pointer; }
h2, h3, p { margin: 0; }
@media (max-width: 650px) { .comparison { grid-template-columns: 1fr; } .document-text { height: 16rem; } }
</style>
