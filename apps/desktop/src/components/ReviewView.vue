<script setup lang="ts">
import { OnyxButton, OnyxCard, OnyxCheckbox, OnyxHeadline, OnyxSelect, OnyxTag, OnyxTextarea } from "sit-onyx";
import { computed, nextTick, ref, watch } from "vue";
import type { useReview } from "../composables/useReview";
import { entityLabel, entityOption } from "../lib/entityLabels";
import { codePointOffset, selectionOffsets } from "../lib/selection";
import { projectReview, previewSelection, type ReviewSegment } from "../lib/reviewProjection";

const props = withDefaults(defineProps<{ review: ReturnType<typeof useReview>; pairName: string; entityTypes?: string[]; disabled?: boolean }>(), { disabled: false, entityTypes: () => [] });
const emit = defineEmits<{ back: [] }>();
const entity = ref("PERSON"), keyboard = ref(false), page = ref(0);
const selected = ref<{ start: number; end: number } | null>(null);
const original = ref<HTMLElement>(), output = ref<HTMLElement>(), textarea = ref<HTMLTextAreaElement>();
const details = ref<HTMLDetailsElement>(), focused = ref<string | null>(null);
const controlsBusy = computed(() => props.disabled || props.review.busy.value);
const statuses = { pending: "Ausstehend", approved: "Freigegeben", rejected: "Abgelehnt", "needs-rework": "Nacharbeit erforderlich" };
const errors: Record<string, string> = {
  review_conflict: "Das gespeicherte Ergebnis wurde geändert. Ihre Änderungen bleiben erhalten. Verwerfen Sie sie nur, wenn Sie den aktuellen Stand neu öffnen möchten.",
  review_mismatch: "Die gespeicherte Prüfung stimmt nicht mit der aktuellen Ausgabe überein. Kehren Sie zur Dokumentliste zurück und verarbeiten Sie das Dokument erneut.",
  output_conflict: "Die Ausgabedatei wurde geändert. Ihre Änderungen bleiben erhalten; prüfen Sie den Konflikt in der Dokumentliste.",
  reprocess_required: "Quelle oder Erkennung hat sich geändert. Das Dokument muss erneut verarbeitet werden.",
  operation_busy: "Eine andere Verarbeitung läuft. Versuchen Sie es danach erneut.",
};
const warnings: Record<string, string> = {
  empty_document: "Keine extrahierbaren Inhalte. Eine Freigabe ist nicht möglich.",
  headers_footers: "Kopf- oder Fußzeilen wurden nicht übernommen.",
};
const detections = computed(() => [...props.review.active.value].sort((a, b) => a.start - b.start || a.end - b.end || a.id.localeCompare(b.id)));
const entities = computed(() => [...new Set([...props.entityTypes, ...detections.value.map(item => item.entity_type), "CUSTOM"])]
  .sort().map(entityOption));
const visibleDetections = computed(() => detections.value.slice(page.value * 20, (page.value + 1) * 20));
const sourcePoints = computed(() => Array.from(props.review.data.value?.original_text ?? ""));
const projection = computed(() => projectReview(props.review.data.value?.original_text ?? "", props.review.active.value));
watch(detections, () => {
  page.value = Math.min(page.value, Math.max(0, Math.ceil(detections.value.length / 20) - 1));
  selected.value = null;
  if (!detections.value.some(item => item.id === focused.value)) focused.value = null;
});
watch(() => props.review.data.value, () => { selected.value = null; });
watch(() => props.review.key.value, () => { focused.value = null; page.value = 0; });
// Updating a textarea value moves its caret; keep the reader at the current passage.
watch(() => projection.value.text, async () => {
  const control = textarea.value;
  if (!control) return;
  const { scrollTop, scrollLeft, selectionStart } = control;
  await nextTick();
  control.setSelectionRange(selectionStart, selectionStart);
  control.scrollTop = scrollTop; control.scrollLeft = scrollLeft;
});
function excerpt(start: number, end: number) {
  return sourcePoints.value.slice(start, Math.min(end, start + 100)).join("") + (end - start > 100 ? "…" : "");
}
function isFocused(part: ReviewSegment) { return focused.value !== null && part.ids.includes(focused.value); }
async function showDetection(id: string, fromText = false, event?: Event) {
  if (event?.type === "click" && !window.getSelection()?.isCollapsed) return;
  const index = detections.value.findIndex(item => item.id === id);
  if (index < 0) return;
  focused.value = id; selected.value = null;
  page.value = Math.floor(index / 20);
  if (details.value) details.value.open = true;
  if (!fromText) keyboard.value = false;
  await nextTick();
  if (fromText) {
    const row = details.value?.querySelector<HTMLElement>(`[data-row="${index % 20}"]`);
    row?.scrollIntoView({ block: "nearest" }); row?.focus({ preventScroll: true });
  } else {
    for (const container of [original.value, output.value]) {
      const mark = container?.querySelector<HTMLElement>(".focused-redaction");
      mark?.scrollIntoView({ block: "center" });
    }
    output.value?.querySelector<HTMLElement>(".focused-redaction")?.focus({ preventScroll: true });
  }
}
// Textareas collapse CRLF to LF. Cache the native boundary after each collapsed pair.
const collapsedNewlines = computed(() => {
  const boundaries: number[] = [];
  for (const match of projection.value.text.matchAll(/\r\n/g)) {
    boundaries.push(match.index + 1 - boundaries.length);
  }
  return boundaries;
});
function previewOffset(nativeOffset: number) {
  const boundaries = collapsedNewlines.value;
  let low = 0, high = boundaries.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (boundaries[middle]! <= nativeOffset) low = middle + 1;
    else high = middle;
  }
  return codePointOffset(projection.value.text, nativeOffset + low);
}
async function keyboardSelection() {
  keyboard.value = !keyboard.value; selected.value = null;
  await nextTick();
  if (keyboard.value) { textarea.value?.setSelectionRange(0, 0); textarea.value?.focus(); }
  else output.value?.focus();
}
function capture(view: "original" | "preview") {
  selected.value = null;
  if (view === "preview" && keyboard.value && textarea.value) {
    const control = textarea.value;
    if (control.selectionStart === control.selectionEnd) return;
    try { selected.value = previewSelection(projection.value.preview, previewOffset(control.selectionStart), previewOffset(control.selectionEnd)); }
    catch { /* A partial surrogate pair is not a selectable redaction. */ }
  } else {
    const selection = window.getSelection(), container = view === "original" ? original.value : output.value;
    const span = container && selection ? selectionOffsets(container, selection) : null;
    selected.value = span && view === "preview" ? previewSelection(projection.value.preview, span.start, span.end) : span;
  }
}
function add() {
  if (!selected.value) return;
  props.review.add(selected.value, entity.value); selected.value = null;
  if (!keyboard.value) window.getSelection()?.removeAllRanges();
}
function restorePreview(event: Event) {
  (event.target as HTMLTextAreaElement).value = projection.value.text;
  selected.value = null;
}
</script>

<template>
  <OnyxCard class="review" role="region" aria-labelledby="review-heading">
    <div class="actions review-header">
      <OnyxHeadline id="review-heading" is="h2">{{ pairName }} · {{ review.key.value?.doc_id }}</OnyxHeadline>
      <OnyxButton label="Zur Dokumentliste" type="button" mode="outline" :disabled="controlsBusy" @click="emit('back')" />
    </div>
    <p v-if="review.busy.value" role="status">Prüfung wird geladen oder gespeichert …</p>
    <p v-if="review.error.value" role="alert">{{ errors[review.error.value.code] ?? "Die Prüfung konnte nicht geladen oder gespeichert werden. Ihre ungespeicherten Änderungen bleiben erhalten." }}</p>
    <p aria-live="polite">{{ review.message.value }}</p>
    <template v-if="review.data.value">
      <div class="review-status"><OnyxTag :label="statuses[review.status.value]" :color="review.status.value === 'approved' ? 'success' : review.status.value === 'rejected' ? 'danger' : review.status.value === 'needs-rework' ? 'warning' : 'neutral'" /><span>{{ review.dirty.value ? 'Ungespeicherte Änderungen' : 'Gespeicherter Stand' }}</span></div>
      <section v-if="review.data.value.warnings.length" class="warnings" aria-label="Hinweise zur Extraktion">
        <h3>Inhalte möglicherweise unvollständig</h3>
        <OnyxCheckbox v-for="warning in review.data.value.warnings" :key="warning" :value="warning"
          :label="`${warnings[warning] ?? `Nicht übernommener Inhalt (${warning})`} Hinweis geprüft.`"
          :model-value="review.acknowledged.value.includes(warning)" :disabled="controlsBusy"
          @update:model-value="checked => review.acknowledged.value = checked ? [...review.acknowledged.value, warning] : review.acknowledged.value.filter(code => code !== warning)" />
      </section>
      <div class="actions">
        <OnyxButton data-testid="keyboard-select" :label="keyboard ? 'Markierungen anzeigen' : 'Per Tastatur auswählen'" type="button" mode="outline" :disabled="controlsBusy" @click="keyboardSelection" />
        <OnyxSelect v-model="entity" label="Typ der neuen Schwärzung" list-label="Entitätstypen" :options="entities" :hide-clear-icon="true" :disabled="controlsBusy" />
        <OnyxButton data-testid="add-redaction" label="Auswahl schwärzen" type="button" :disabled="controlsBusy || !selected" @click="add" />
        <OnyxButton data-testid="undo-review" label="Korrektur zurücknehmen" type="button" mode="outline" :disabled="controlsBusy || !review.canUndo.value" @click="review.undo" />
      </div>
      <p v-if="selected" role="status">Auswahl: Position {{ selected.start }}–{{ selected.end }} · „{{ excerpt(selected.start, selected.end) }}“</p>
      <p v-if="keyboard" id="selection-help">Wählen Sie Text mit Umschalt- und Pfeiltasten aus. Navigieren Sie anschließend mit Umschalt+Tab rückwärts bis zu „Auswahl schwärzen“. Die Vorschau ist schreibgeschützt. Platzhalter werden als ganze Schwärzung ausgewählt.</p>
      <div class="comparison">
        <section aria-label="Originaltext">
          <h3>Originaltext</h3>
          <div ref="original" data-testid="review-original" class="document-text" tabindex="0" aria-label="Originaltext mit Markierungen" @mouseup="capture('original')" @keyup="capture('original')"><template v-for="(part, index) in projection.original" :key="index"><mark v-if="part.ids.length" :class="{ 'focused-redaction': isFocused(part) }" tabindex="0" role="button" aria-label="Schwärzung in der Liste anzeigen" @click="showDetection(part.ids[0]!, true, $event)" @keydown.enter.prevent="showDetection(part.ids[0]!, true)" @keydown.space.prevent="showDetection(part.ids[0]!, true)">{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></div>
        </section>
        <section aria-label="Geschwärzte Vorschau">
          <h3>Geschwärzte Vorschau</h3>
          <!-- WebKit's readonly textarea blocks caret navigation; prevent edits while retaining native selection. -->
          <textarea v-if="keyboard" ref="textarea" data-testid="selection-text" class="document-text" aria-readonly="true" spellcheck="false" autocomplete="off" aria-label="Text in der geschwärzten Vorschau auswählen" aria-describedby="selection-help" :value="projection.text" :disabled="controlsBusy" @beforeinput.prevent @paste.prevent @drop.prevent @input="restorePreview" @select="capture('preview')" @keyup="capture('preview')" @mouseup="capture('preview')" />
          <div v-else ref="output" data-testid="review-output" class="document-text" tabindex="0" aria-label="Geschwärzte Vorschau auswählen" @mouseup="capture('preview')" @keyup="capture('preview')"><template v-for="(part, index) in projection.preview" :key="index"><mark v-if="part.ids.length" :class="{ 'focused-redaction': isFocused(part) }" tabindex="0" role="button" aria-label="Schwärzung in der Liste anzeigen" @click="showDetection(part.ids[0]!, true, $event)" @keydown.enter.prevent="showDetection(part.ids[0]!, true)" @keydown.space.prevent="showDetection(part.ids[0]!, true)">{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></div>
        </section>
      </div>
      <p v-if="review.decisionsChanged.value">Ungespeicherte Vorschau – Speichern übernimmt die Änderungen.</p>
      <p>Fehlenden Text in der Vorschau auswählen und schwärzen. Markierte Stellen anklicken, um den Listeneintrag anzuzeigen.</p>
      <details ref="details">
        <summary>Schwärzungen bearbeiten ({{ detections.length }})</summary>
        <div class="details-body">
          <p>Konfidenz ist eine Einschätzung des Detektors und keine Datenschutzgarantie.</p>
          <ul class="detections">
            <li v-for="(item, index) in visibleDetections" :key="item.id" :data-testid="`detection-${item.id}`" :data-row="index" :class="{ 'focused-redaction': focused === item.id }" tabindex="-1">
              <div class="detection-text"><q>{{ excerpt(item.start, item.end) }}</q><small>{{ entityLabel(item.entity_type) }} ({{ item.entity_type }}) · {{ item.origin === 'manual' ? 'Manuell' : `Konfidenz ${item.confidence === null ? 'unbekannt' : Math.round(item.confidence * 100) + ' %'}` }}</small></div>
              <OnyxButton :data-testid="`jump-${item.id}`" label="Im Text anzeigen" type="button" mode="outline" :disabled="controlsBusy" @click="showDetection(item.id)" />
              <OnyxSelect :model-value="item.entity_type" :label="`Typ für ${excerpt(item.start, item.end)}`" list-label="Entitätstypen" :options="entities" :hide-clear-icon="true" :disabled="controlsBusy" @update:model-value="value => value && review.changeType(item.id, value as string)" />
              <OnyxButton :label="`Schwärzung entfernen: ${excerpt(item.start, item.end)}`" type="button" mode="outline" :disabled="controlsBusy" @click="review.dismiss(item.id)" />
            </li>
          </ul>
          <div v-if="detections.length > 20" class="actions">
            <OnyxButton label="Vorherige Schwärzungen" type="button" mode="outline" :disabled="controlsBusy || page === 0" @click="page--" />
            <span>Seite {{ page + 1 }} von {{ Math.ceil(detections.length / 20) }}</span>
            <OnyxButton label="Weitere Schwärzungen" type="button" mode="outline" :disabled="controlsBusy || (page + 1) * 20 >= detections.length" @click="page++" />
          </div>
        </div>
      </details>
      <OnyxTextarea v-model="review.notes.value" label="Private Prüfnotizen" :disabled="controlsBusy" />
      <div class="actions">
        <OnyxButton data-testid="save-review" label="Prüfung speichern" type="button" :disabled="controlsBusy || !review.dirty.value" @click="review.save()" />
        <OnyxButton data-testid="approve-review" label="Freigeben" type="button" :disabled="controlsBusy || !review.canApprove.value" @click="review.save('approved')" />
        <OnyxButton label="Ablehnen" type="button" mode="outline" :disabled="controlsBusy" @click="review.save('rejected')" />
        <OnyxButton label="Nacharbeit erforderlich" type="button" mode="outline" :disabled="controlsBusy" @click="review.save('needs-rework')" />
        <OnyxButton label="Als ausstehend speichern" type="button" mode="outline" :disabled="controlsBusy" @click="review.save('pending')" />
      </div>
      <p v-if="review.decisionsChanged.value">Korrekturen zunächst speichern, danach die aktualisierte Ausgabe prüfen und ausdrücklich freigeben.</p>
    </template>
  </OnyxCard>
</template>

<style scoped>
.review, .warnings, .detections { display: grid; gap: var(--onyx-spacing-md); min-width: 0; }
.actions, .detections li { display: flex; align-items: flex-end; flex-wrap: wrap; gap: var(--onyx-spacing-md); }
.actions h2 { flex: 1 1 18rem; }
.review-status { display: flex; align-items: center; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
.review-status > span { color: var(--onyx-color-text-icons-neutral-medium); }
.actions :deep(.onyx-select) { max-width: 18rem; }
.comparison { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--onyx-spacing-md); }
.comparison section { display: grid; gap: var(--onyx-spacing-sm); min-width: 0; }
.document-text { display: block; white-space: pre-wrap; overflow-wrap: anywhere; width: 100%; height: clamp(22rem, 48vh, 38rem); overflow: auto; padding: var(--onyx-spacing-sm); border: 1px solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-sm); background: var(--onyx-color-base-background-blank); color: inherit; font: inherit; box-sizing: border-box; }
textarea.document-text { resize: vertical; }
.document-text:focus-visible, input:focus-visible, summary:focus-visible { outline: 3px solid var(--onyx-color-text-icons-primary-intense); outline-offset: 2px; }
mark { cursor: pointer; color: var(--onyx-color-text-icons-warning-intense); background: var(--onyx-color-base-warning-200); text-decoration: underline; }
mark:focus-visible, .focused-redaction { outline: var(--onyx-outline-width) solid var(--onyx-color-component-focus-primary); outline-offset: 2px; }
mark.focused-redaction { background: var(--onyx-color-base-warning-300); }
.detections { margin: 0; padding: var(--onyx-spacing-xs); list-style: none; }
.detections li { padding: var(--onyx-spacing-md); border: 1px solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-sm); }
.detection-text { flex: 1 1 12rem; min-width: 0; overflow-wrap: anywhere; }
.detection-text q, .detection-text small { display: block; }
.detection-text small { margin-block-start: var(--onyx-spacing-xs); color: var(--onyx-color-text-icons-neutral-medium); }
.details-body { display: grid; gap: var(--onyx-spacing-md); padding-block-start: var(--onyx-spacing-md); }
.warnings { border-inline-start: 4px solid var(--onyx-color-text-icons-warning-intense); padding: var(--onyx-spacing-sm); }
summary { cursor: pointer; font-weight: var(--onyx-font-weight-semibold); }
h2, h3, p { margin: 0; }
@media (max-width: 768px) {
  .comparison { grid-template-columns: 1fr; }
  .document-text { height: 16rem; }
  .review-header { align-items: flex-start; flex-direction: column; }
  .review-header h2 { flex-basis: auto; }
}
</style>
