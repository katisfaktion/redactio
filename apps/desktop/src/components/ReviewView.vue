<script setup lang="ts">
import { OnyxButton, OnyxCard, OnyxCheckbox, OnyxHeadline, OnyxSelect, OnyxTag, OnyxTextarea } from "sit-onyx";
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import type { useReview } from "../composables/useReview";
import { entityLabel, entityOption } from "../lib/entityLabels";
import { codePointOffset, selectionOffsets } from "../lib/selection";
import { projectReview, previewSelection, type ReviewSegment } from "../lib/reviewProjection";

const props = withDefaults(defineProps<{ review: ReturnType<typeof useReview>; pairName: string; entityTypes?: string[]; disabled?: boolean }>(), { disabled: false, entityTypes: () => [] });
const emit = defineEmits<{ back: [] }>();
const entity = ref("PERSON"), keyboard = ref<"original" | "preview" | null>(null), page = ref(0);
const selected = ref<{ start: number; end: number } | null>(null);
const original = ref<HTMLElement>(), output = ref<HTMLElement>(), textarea = ref<HTMLTextAreaElement>();
const originalInput = ref<HTMLTextAreaElement>(), focused = ref<string | null>(null);
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
const focusedIndex = computed(() => detections.value.findIndex(item => item.id === focused.value));
const focusedDetection = computed(() => detections.value[focusedIndex.value]);
const overlaps = computed(() => selected.value ? detections.value.filter(item => item.start < selected.value!.end && item.end > selected.value!.start) : []);
const uncovered = computed(() => {
  if (!selected.value) return [];
  const { start, end } = selected.value;
  // Only show text that will actually become visible, accounting for retained overlaps.
  const retained = detections.value.filter(item => !overlaps.value.includes(item));
  return projectReview(props.review.data.value?.original_text ?? "", retained).original
    .filter(part => !part.ids.length).flatMap(part => overlaps.value.flatMap(old => [
      { start: Math.max(part.start, old.start), end: Math.min(part.end, old.end, start) },
      { start: Math.max(part.start, old.start, end), end: Math.min(part.end, old.end) },
    ])).filter(span => span.start < span.end)
    .filter((span, index, all) => all.findIndex(other => other.start === span.start && other.end === span.end) === index);
});
const applyLabel = computed(() => overlaps.value.length
  ? `${overlaps.value.length} ${overlaps.value.length === 1 ? "Schwärzung" : "Schwärzungen"} ersetzen` : "Auswahl schwärzen");
function isFocused(part: ReviewSegment) { return focused.value !== null && part.ids.includes(focused.value); }
async function showDetection(id: string, fromText = false, event?: Event) {
  if (controlsBusy.value || (event?.type === "click" && !window.getSelection()?.isCollapsed)) return;
  const index = detections.value.findIndex(item => item.id === id);
  if (index < 0) return;
  focused.value = id; selected.value = null;
  page.value = Math.floor(index / 20);
  entity.value = detections.value[index]!.entity_type;
  if (fromText) return; // Inspect in place; never take focus away from a pointer selection.
  keyboard.value = null;
  await nextTick();
  for (const container of [original.value, output.value]) {
    const mark = container?.querySelector<HTMLElement>(".focused-redaction");
    if (!container || !mark) continue;
    const top = mark.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop;
    container.scrollTop = Math.max(0, top - container.clientHeight / 2);
  }
}
function stepDetection(direction: number) {
  const index = focusedIndex.value < 0 ? 0 : focusedIndex.value + direction;
  const next = detections.value[index];
  if (next) void showDetection(next.id);
}
function changeType(value: unknown) {
  if (controlsBusy.value || !focusedDetection.value || typeof value !== "string" || !value) return;
  const id = props.review.changeType(focusedDetection.value.id, value);
  if (id) focused.value = id;
}
async function editRange() {
  const item = focusedDetection.value;
  if (!item || controlsBusy.value) return;
  const scroll = original.value?.scrollTop ?? originalInput.value?.scrollTop ?? 0;
  keyboard.value = "original"; entity.value = item.entity_type;
  selected.value = { start: item.start, end: item.end };
  await nextTick();
  const input = originalInput.value;
  if (!input) return;
  const native = (offset: number) => sourcePoints.value.slice(0, offset).join("").replace(/\r\n/g, "\n").length;
  input.focus({ preventScroll: true });
  input.setSelectionRange(native(item.start), native(item.end));
  input.scrollTop = scroll;
}
// Native highlights keep a pointer-selected range visible while using the side panel.
function clearHighlight() { globalThis.CSS?.highlights?.delete("redactio-selection"); }
watch(selected, async span => {
  clearHighlight();
  if (!span || !globalThis.CSS?.highlights || typeof Highlight === "undefined") return;
  await nextTick();
  if (!original.value || selected.value !== span) return;
  const range = document.createRange();
  const start = sourcePoints.value.slice(0, span.start).join("").length;
  const end = sourcePoints.value.slice(0, span.end).join("").length;
  const walker = document.createTreeWalker(original.value, NodeFilter.SHOW_TEXT);
  let offset = 0, foundStart = false;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const length = node.textContent?.length ?? 0;
    if (!foundStart && start <= offset + length) { range.setStart(node, start - offset); foundStart = true; }
    if (end <= offset + length) {
      range.setEnd(node, end - offset);
      CSS.highlights.set("redactio-selection", new Highlight(range));
      break;
    }
    offset += length;
  }
});
onBeforeUnmount(clearHighlight);
// Textareas collapse CRLF to LF. Cache the native boundary after each collapsed pair.
const keyboardText = computed(() => keyboard.value === "original" ? props.review.data.value?.original_text ?? "" : projection.value.text);
const collapsedNewlines = computed(() => {
  const boundaries: number[] = [];
  for (const match of keyboardText.value.matchAll(/\r\n/g)) {
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
  return codePointOffset(keyboardText.value, nativeOffset + low);
}
async function keyboardSelection() {
  const previous = keyboard.value;
  const scroll = (previous === "original" ? originalInput.value : previous === "preview" ? textarea.value : output.value)?.scrollTop ?? 0;
  keyboard.value = keyboard.value ? null : "preview"; selected.value = null;
  await nextTick();
  const control = keyboard.value ? textarea.value : previous === "original" ? original.value : output.value;
  control?.focus({ preventScroll: true });
  if (control) control.scrollTop = scroll;
}
function capture(view: "original" | "preview") {
  if (controlsBusy.value) return;
  selected.value = null;
  const control = view === "original" ? originalInput.value : textarea.value;
  if (keyboard.value === view && control) {
    if (control.selectionStart === control.selectionEnd) return;
    try {
      const start = previewOffset(control.selectionStart), end = previewOffset(control.selectionEnd);
      selected.value = view === "original" ? { start, end } : previewSelection(projection.value.preview, start, end);
    } catch { /* A partial surrogate pair is not a selectable redaction. */ }
  } else {
    const selection = window.getSelection(), container = view === "original" ? original.value : output.value;
    const span = container && selection ? selectionOffsets(container, selection) : null;
    selected.value = span && view === "preview" ? previewSelection(projection.value.preview, span.start, span.end) : span;
  }
}
function add() {
  if (!selected.value || controlsBusy.value) return;
  const id = props.review.replace(selected.value, entity.value);
  if (!id) return;
  focused.value = id; selected.value = null;
  if (!keyboard.value) window.getSelection()?.removeAllRanges();
}
function restoreText(event: Event, value: string) {
  (event.target as HTMLTextAreaElement).value = value;
  selected.value = null;
}
</script>

<template>
  <OnyxCard class="review" role="region" aria-labelledby="review-heading">
    <div class="actions review-header">
      <div>
        <OnyxHeadline id="review-heading" is="h1">{{ pairName }} · {{ review.key.value?.doc_id }}</OnyxHeadline>
        <div class="review-status"><OnyxTag :label="statuses[review.status.value]" :color="review.status.value === 'approved' ? 'success' : review.status.value === 'rejected' ? 'danger' : review.status.value === 'needs-rework' ? 'warning' : 'neutral'" /><span>{{ review.dirty.value ? 'Ungespeicherte Änderungen' : 'Gespeicherter Stand' }}</span></div>
      </div>
      <OnyxButton label="Zur Dokumentliste" type="button" mode="outline" :disabled="controlsBusy" @click="emit('back')" />
    </div>
    <p v-if="review.busy.value" role="status">Prüfung wird geladen oder gespeichert …</p>
    <p v-if="review.error.value" role="alert">{{ errors[review.error.value.code] ?? "Die Prüfung konnte nicht geladen oder gespeichert werden. Ihre ungespeicherten Änderungen bleiben erhalten." }}</p>
    <template v-if="review.data.value">
      <section v-if="review.data.value.warnings.length" class="warnings" aria-label="Hinweise zur Extraktion">
        <h3>Inhalte möglicherweise unvollständig</h3>
        <OnyxCheckbox v-for="warning in review.data.value.warnings" :key="warning" :value="warning"
          :label="`${warnings[warning] ?? `Nicht übernommener Inhalt (${warning})`} Hinweis geprüft.`"
          :model-value="review.acknowledged.value.includes(warning)" :disabled="controlsBusy"
          @update:model-value="checked => review.acknowledged.value = checked ? [...review.acknowledged.value, warning] : review.acknowledged.value.filter(code => code !== warning)" />
      </section>
      <div class="workspace">
        <div class="comparison">
          <section aria-label="Originaltext">
            <h3>Originaltext</h3>
            <textarea v-if="keyboard === 'original'" ref="originalInput" data-testid="range-text" class="document-text" aria-readonly="true" spellcheck="false" autocomplete="off" aria-label="Bereich im Original korrigieren" aria-describedby="selection-help" :value="review.data.value.original_text" :disabled="controlsBusy" @beforeinput.prevent @paste.prevent @drop.prevent @input="restoreText($event, review.data.value.original_text)" @select="capture('original')" @keyup="capture('original')" @mouseup="capture('original')" />
            <div v-else ref="original" data-testid="review-original" class="document-text" tabindex="0" aria-label="Originaltext mit Markierungen" @mouseup="capture('original')" @keyup="capture('original')"><template v-for="(part, index) in projection.original" :key="index"><mark v-if="part.ids.length" :class="{ 'focused-redaction': isFocused(part) }" tabindex="0" role="button" aria-label="Details zur Schwärzung anzeigen" @click="showDetection(part.ids[0]!, true, $event)" @keydown.enter.prevent="showDetection(part.ids[0]!, true)" @keydown.space.prevent="showDetection(part.ids[0]!, true)" @keyup.enter.stop @keyup.space.stop>{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></div>
          </section>
          <section aria-label="Geschwärzte Vorschau">
            <h3>Geschwärzte Vorschau</h3>
            <!-- WebKit's readonly textarea blocks caret navigation; prevent edits while retaining native selection. -->
            <textarea v-if="keyboard === 'preview'" ref="textarea" data-testid="selection-text" class="document-text" aria-readonly="true" spellcheck="false" autocomplete="off" aria-label="Text in der geschwärzten Vorschau auswählen" aria-describedby="selection-help" :value="projection.text" :disabled="controlsBusy" @beforeinput.prevent @paste.prevent @drop.prevent @input="restoreText($event, projection.text)" @select="capture('preview')" @keyup="capture('preview')" @mouseup="capture('preview')" />
            <div v-else ref="output" data-testid="review-output" class="document-text" tabindex="0" aria-label="Geschwärzte Vorschau auswählen" @mouseup="capture('preview')" @keyup="capture('preview')"><template v-for="(part, index) in projection.preview" :key="index"><mark v-if="part.ids.length" :class="{ 'focused-redaction': isFocused(part) }" tabindex="0" role="button" aria-label="Details zur Schwärzung anzeigen" @click="showDetection(part.ids[0]!, true, $event)" @keydown.enter.prevent="showDetection(part.ids[0]!, true)" @keydown.space.prevent="showDetection(part.ids[0]!, true)" @keyup.enter.stop @keyup.space.stop>{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></div>
          </section>
        </div>
        <aside data-testid="redaction-inspector" class="inspector" aria-labelledby="inspector-heading">
          <h2 id="inspector-heading">Schwärzungen</h2>
          <div class="actions detection-navigation">
            <OnyxButton data-testid="previous-redaction" label="Vorherige" aria-label="Vorherige Schwärzung" type="button" mode="outline" :disabled="controlsBusy || focusedIndex <= 0" @click="stepDetection(-1)" />
            <span>{{ focusedIndex < 0 ? detections.length + ' gesamt' : (focusedIndex + 1) + ' von ' + detections.length }}</span>
            <OnyxButton data-testid="next-redaction" label="Nächste" aria-label="Nächste Schwärzung" type="button" mode="outline" :disabled="controlsBusy || !detections.length || focusedIndex === detections.length - 1" @click="stepDetection(1)" />
          </div>
          <section v-if="selected" class="inspector-section" aria-label="Auswahl korrigieren">
            <h3>Auswahl</h3>
            <q class="excerpt">{{ excerpt(selected.start, selected.end) }}</q>
            <small role="status">Auswahl: Position {{ selected.start }}–{{ selected.end }}</small>
            <p v-if="overlaps.length">{{ overlaps.length }} bestehende {{ overlaps.length === 1 ? 'Schwärzung wird' : 'Schwärzungen werden' }} vollständig ersetzt:</p>
            <ul v-if="overlaps.length" class="affected"><li v-for="item in overlaps" :key="item.id">„{{ excerpt(item.start, item.end) }}“ · {{ entityLabel(item.entity_type) }}</li></ul>
            <div v-if="uncovered.length"><p>Danach wieder sichtbar:</p><ul class="affected"><li v-for="span in uncovered" :key="`${span.start}:${span.end}`"><q>{{ excerpt(span.start, span.end) }}</q></li></ul></div>
          </section>
          <section v-else-if="focusedDetection" class="inspector-section" aria-label="Ausgewählte Schwärzung">
            <q class="excerpt">{{ excerpt(focusedDetection.start, focusedDetection.end) }}</q>
            <small>{{ entityLabel(focusedDetection.entity_type) }} ({{ focusedDetection.entity_type }}) · {{ focusedDetection.origin === 'manual' ? 'Manuell' : `Konfidenz ${focusedDetection.confidence === null ? 'unbekannt' : Math.round(focusedDetection.confidence * 100) + ' %'}` }}</small>
            <OnyxSelect :model-value="focusedDetection.entity_type" label="Typ der Schwärzung" list-label="Entitätstypen" :options="entities" :hide-clear-icon="true" :disabled="controlsBusy" @update:model-value="changeType" />
            <OnyxButton data-testid="edit-range" label="Bereich korrigieren" type="button" mode="outline" :disabled="controlsBusy" @click="editRange" />
            <OnyxButton label="Schwärzung entfernen" type="button" mode="outline" :disabled="controlsBusy" @click="review.dismiss(focusedDetection.id)" />
          </section>
          <p v-else>Markierte Stelle anklicken oder Text auswählen, um eine Schwärzung zu korrigieren.</p>
          <div v-show="selected || !focusedDetection" class="inspector-section">
            <OnyxSelect v-model="entity" label="Typ der neuen Schwärzung" list-label="Entitätstypen" :options="entities" :hide-clear-icon="true" :disabled="controlsBusy" />
            <OnyxButton data-testid="add-redaction" :label="applyLabel" type="button" :disabled="controlsBusy || !selected" @click="add" />
          </div>
          <OnyxButton data-testid="keyboard-select" :label="keyboard ? 'Markierungen anzeigen' : 'Per Tastatur auswählen'" type="button" mode="outline" :disabled="controlsBusy" @click="keyboardSelection" />
          <p v-if="keyboard" id="selection-help">{{ keyboard === 'original' ? 'Im Original den genauen Bereich neu auswählen.' : 'In der Vorschau werden Platzhalter als ganze Schwärzung ausgewählt.' }} Umschalt- und Pfeiltasten ändern die Auswahl; mit Tab erreichen Sie die Korrektur. Der Text ist schreibgeschützt.</p>
          <OnyxButton data-testid="undo-review" label="Korrektur zurücknehmen" type="button" mode="outline" :disabled="controlsBusy || !review.canUndo.value" @click="review.undo" />
          <p class="edit-status" aria-live="polite">{{ review.message.value }}</p>
          <details>
            <summary>Alle Schwärzungen ({{ detections.length }})</summary>
            <ul class="detections">
              <li v-for="item in visibleDetections" :key="item.id" :data-testid="`detection-${item.id}`" :class="{ 'focused-redaction': focused === item.id }">
                <button :data-testid="`jump-${item.id}`" type="button" :disabled="controlsBusy" @click="showDetection(item.id)"><q>{{ excerpt(item.start, item.end) }}</q><small>{{ entityLabel(item.entity_type) }} ({{ item.entity_type }})</small></button>
              </li>
            </ul>
            <div v-if="detections.length > 20" class="actions">
              <OnyxButton label="Vorherige Seite" type="button" mode="outline" :disabled="controlsBusy || page === 0" @click="page--" />
              <span>Seite {{ page + 1 }} von {{ Math.ceil(detections.length / 20) }}</span>
              <OnyxButton label="Weitere Seite" type="button" mode="outline" :disabled="controlsBusy || (page + 1) * 20 >= detections.length" @click="page++" />
            </div>
          </details>
        </aside>
      </div>
      <div class="review-footer">
        <p class="draft-status">{{ review.decisionsChanged.value ? 'Ungespeicherte Vorschau – Speichern übernimmt die Änderungen.' : 'Korrekturen speichern, anschließend die Ausgabe prüfen und ausdrücklich freigeben.' }}</p>
        <div class="actions">
          <OnyxButton data-testid="save-review" label="Prüfung speichern" type="button" :disabled="controlsBusy || !review.dirty.value" @click="review.save()" />
          <OnyxButton data-testid="approve-review" label="Freigeben" type="button" :disabled="controlsBusy || !review.canApprove.value" @click="review.save('approved')" />
          <details class="additional-actions"><summary>Notizen und Prüfstatus</summary><div class="inspector-section">
            <OnyxTextarea v-model="review.notes.value" label="Private Prüfnotizen" :disabled="controlsBusy" />
            <OnyxButton label="Ablehnen" type="button" mode="outline" :disabled="controlsBusy" @click="review.save('rejected')" />
            <OnyxButton label="Nacharbeit erforderlich" type="button" mode="outline" :disabled="controlsBusy" @click="review.save('needs-rework')" />
            <OnyxButton label="Als ausstehend speichern" type="button" mode="outline" :disabled="controlsBusy" @click="review.save('pending')" />
          </div></details>
        </div>
      </div>
    </template>
  </OnyxCard>
</template>

<style scoped>
.review, .warnings, .review-footer { display: grid; gap: var(--onyx-spacing-md); min-width: 0; }
.actions { display: flex; align-items: center; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
.review-header { justify-content: space-between; }
.review-header > div { display: grid; gap: var(--onyx-spacing-sm); min-width: 0; }
.review-header h1 { font-size: var(--onyx-font-size-xl); overflow-wrap: anywhere; }
.review-status { display: flex; align-items: center; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
.review-status > span, small, .edit-status { color: var(--onyx-color-text-icons-neutral-medium); }
.workspace { display: grid; grid-template-columns: minmax(0, 1fr) 20rem; gap: var(--onyx-spacing-md); min-width: 0; align-items: start; }
.comparison { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: var(--onyx-spacing-md); min-width: 0; }
.comparison section { display: grid; grid-template-rows: auto minmax(0, 1fr); gap: var(--onyx-spacing-sm); min-width: 0; }
.document-text { display: block; white-space: pre-wrap; overflow-wrap: anywhere; width: 100%; height: clamp(22rem, 52vh, 50rem); overflow: auto; overflow-anchor: none; padding: var(--onyx-spacing-sm); border: 1px solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-sm); background: var(--onyx-color-base-background-blank); color: inherit; font: inherit; box-sizing: border-box; scrollbar-gutter: stable; }
textarea.document-text { resize: none; }
.document-text:focus-visible, summary:focus-visible, .detections button:focus-visible { outline: 3px solid var(--onyx-color-text-icons-primary-intense); outline-offset: 2px; }
mark { cursor: text; color: var(--onyx-color-text-icons-warning-intense); background: var(--onyx-color-base-warning-200); text-decoration: underline; }
mark:focus-visible, .focused-redaction { outline: var(--onyx-outline-width) solid var(--onyx-color-component-focus-primary); outline-offset: 2px; }
mark.focused-redaction { background: var(--onyx-color-base-warning-300); }
.inspector { display: flex; flex-direction: column; gap: var(--onyx-spacing-md); height: calc(clamp(22rem, 52vh, 50rem) + 2rem); overflow-y: auto; min-width: 0; padding: var(--onyx-spacing-md); border: 1px solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-sm); background: var(--onyx-color-base-background-blank); scrollbar-gutter: stable; }
.inspector > * { flex-shrink: 0; }
.inspector-section { display: grid; gap: var(--onyx-spacing-sm); min-width: 0; }
.inspector .onyx-button { max-width: 100%; }
.inspector h2 { font-size: var(--onyx-font-size-lg); }
.excerpt { display: block; white-space: pre-wrap; overflow-wrap: anywhere; font-weight: var(--onyx-font-weight-semibold); }
.detection-navigation { justify-content: space-between; font-size: var(--onyx-font-size-sm); }
.detection-navigation :deep(.onyx-button) { padding-inline: var(--onyx-spacing-xs); }
.detections { display: grid; gap: var(--onyx-spacing-sm); margin: var(--onyx-spacing-sm) 0; padding: var(--onyx-spacing-xs); list-style: none; }
.detections button { display: block; width: 100%; text-align: start; background: transparent; color: inherit; font: inherit; padding: var(--onyx-spacing-xs); border: 1px solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-sm); cursor: pointer; overflow-wrap: anywhere; }
.detections small { display: block; }
.affected { margin: 0; padding-inline-start: var(--onyx-spacing-md); overflow-wrap: anywhere; }
.warnings { border-inline-start: 4px solid var(--onyx-color-text-icons-warning-intense); padding: var(--onyx-spacing-sm); }
.additional-actions { align-self: center; }
.additional-actions[open] { flex-basis: 100%; }
.additional-actions > div { margin-block-start: var(--onyx-spacing-sm); }
summary { cursor: pointer; font-weight: var(--onyx-font-weight-semibold); }
h1, h2, h3, p { margin: 0; }
@media (max-width: 1100px) {
  .workspace { grid-template-columns: minmax(0, 1fr); grid-template-rows: minmax(10rem, 1fr) 15rem; height: max(28rem, calc(100dvh - 26rem)); align-items: stretch; }
  .comparison { min-height: 0; }
  .inspector { height: 100%; }
  .document-text { height: 100%; min-height: 0; }
}
@media (max-width: 600px) {
  .comparison { grid-template-columns: 1fr; }
  .review-header { align-items: flex-start; }
}
</style>
<style>
::highlight(redactio-selection) { background: var(--onyx-color-base-primary-200); color: var(--onyx-color-text-icons-neutral-intense); text-decoration: underline; }
</style>
