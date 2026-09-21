<script setup lang="ts">
import { OnyxButton, OnyxCard, OnyxCheckbox, OnyxHeadline, OnyxInput, OnyxSelect, OnyxTextarea } from "sit-onyx";
import { computed, ref, watch } from "vue";
import { ProcessingConfigSchema, type CustomRule, type ProcessingConfig, type SyncPair, type ModelInfo, type RulePreview, type SafeError } from "../lib/contracts";
import { entityLabel, entityOption, legacyNativeGroup, supplementaryEntities } from "../lib/entityLabels";

const props = defineProps<{
  pair: SyncPair;
  models: ModelInfo[];
  busy: boolean;
  preview?: RulePreview | null;
  error?: SafeError | null;
  saved?: boolean;
}>();
const emit = defineEmits<{
  save: [pairId: string, config: ProcessingConfig];
  preview: [pairId: string, config: ProcessingConfig, text: string];
}>();
function clone(config: ProcessingConfig): ProcessingConfig { return ProcessingConfigSchema.parse(JSON.parse(JSON.stringify(config))); }
function selectedModel(name: string) { return props.models.find(model => model.name === name); }
const biomedbert = "OpenMed-PII-German-BiomedBERT-Large-340M-v1";
const hugginglil = "pii-sensitive-ner-german";
const legacyModel = "de_core_news_lg";
function nativeDraft(config: ProcessingConfig): ProcessingConfig {
  const model = selectedModel(config.model);
  if (!model?.entity_types || config.model_entities !== null || config.model !== biomedbert) return clone(config);
  const disabledLegacy = new Set(["PERSON", "LOCATION"].filter(type => !config.enabled_entities.includes(type)));
  return { ...clone(config), model_entities: (model.entity_types ?? []).filter(type => !disabledLegacy.has(legacyNativeGroup(type) ?? type)),
    enabled_entities: config.enabled_entities.filter(type => supplementaryEntities.includes(type as typeof supplementaryEntities[number])) };
}
const draft = ref(nativeDraft(props.pair.config));
const modelDrafts = new Map<string, ProcessingConfig>([[draft.value.model, clone(draft.value)]]);
const previewText = ref("");
const visiblePreview = computed(() => props.preview?.pairId === props.pair.id
  && props.preview.text === previewText.value
  && JSON.stringify(props.preview.config) === JSON.stringify(draft.value) ? props.preview : null);
const errors: Record<string, string> = {
  invalid_configuration: "Die Erkennung ist ungültig. Prüfen Sie das Python-RegEx-Muster und die Wortlisten.",
  model_not_found: "Das Sprachmodell ist lokal nicht verfügbar. Wählen Sie ein installiertes Modell.",
  model_incompatible: "Das Sprachmodell ist nicht kompatibel. Wählen Sie ein kompatibles installiertes Modell.",
  invalid_model_manifest: "Die lokale Modellübersicht ist ungültig. Prüfen Sie die Installation.",
  setup_incomplete: "Die lokale Installation ist unvollständig. Bitte prüfen Sie die mitgelieferten Modelle und das Programmpaket.",
  engine_timeout: "Die Prüfung hat das Zeitlimit überschritten. Vereinfachen Sie das Muster oder verkürzen Sie den Testtext.",
  operation_busy: "Eine andere Verarbeitung ist noch aktiv. Versuchen Sie es anschließend erneut.",
  file_busy: "Das Ordnerpaar wird gerade von einer anderen Instanz verwendet.",
  recovery_pending: "Zuerst muss die unterbrochene Verarbeitung mit der bisherigen Erkennung wiederhergestellt werden. Die Einstellungen wurden nicht geändert.",
};
watch([() => props.pair.id, () => JSON.stringify(props.pair.config)], () => {
  draft.value = nativeDraft(props.pair.config);
  modelDrafts.clear(); modelDrafts.set(draft.value.model, clone(draft.value));
  previewText.value = "";
});
watch(() => selectedModel(draft.value.model)?.entity_types, () => {
  if (draft.value.model === biomedbert && draft.value.model_entities === null) {
    const initialized = nativeDraft(draft.value);
    draft.value.model_entities = initialized.model_entities;
    draft.value.enabled_entities = initialized.enabled_entities;
  }
});

const modelOptions = computed(() => {
  const available = props.models.map(model => ({
  value: model.name,
  label: `${model.name === biomedbert ? `BiomedBERT – Deutsch, PII (340M, ${model.version.slice(0, 7)})`
    : model.name === hugginglil ? `HuggingLil – Deutsch, PII (${model.version.slice(0, 7)})`
      : `${model.name} (${model.version})`}${model.compatible ? "" : " – nicht verfügbar"}`,
  disabled: !model.compatible,
  }));
  return available.some(model => model.value === draft.value.model) ? available
    : [{ value: draft.value.model, label: `${draft.value.model} – fehlt`, disabled: true }, ...available];
});
const currentModel = computed(() => selectedModel(draft.value.model));
const modelEntities = computed(() => currentModel.value?.entity_types ?? []);
const entityOptions = computed(() => [...new Set([
  ...modelEntities.value, ...supplementaryEntities, "PERSON", "LOCATION", "CUSTOM",
  ...draft.value.custom_rules.map(rule => rule.entity_type),
])].sort().map(entityOption));
const supplementaryOptions = supplementaryEntities.map(entityOption);
const validation = computed(() => {
  if (!props.models.some(model => model.name === draft.value.model && model.compatible)) {
    return "Das gewählte Modell ist lokal nicht verfügbar oder nicht kompatibel. Wählen Sie ein verfügbares Modell aus.";
  }
  if (draft.value.model !== legacyModel && currentModel.value?.entity_types && draft.value.model_entities === null) {
    return "Eine explizite Auswahl der Modell-Labels ist erforderlich.";
  }
  if (draft.value.custom_rules.some(rule => rule.kind === "regex" ? !rule.pattern.length : !rule.words.length || rule.words.some(word => !word.length))) {
    return "Regeln benötigen ein Muster oder eine Wortliste ohne leere Einträge.";
  }
  return "";
});

function toggleEntity(entity: string, enabled: boolean | null | undefined, target: "model" | "supplementary") {
  const values = target === "model" ? (draft.value.model_entities ?? []) : draft.value.enabled_entities;
  const next = enabled ? [...new Set([...values, entity])] : values.filter(value => value !== entity);
  if (target === "model") draft.value.model_entities = next;
  else draft.value.enabled_entities = next;
}
function chooseModel(model: unknown) {
  if (typeof model !== "string" || model === draft.value.model) return;
  modelDrafts.set(draft.value.model, clone(draft.value));
  const existing = modelDrafts.get(model);
  if (existing) draft.value = clone(existing);
  else {
    const next = clone(draft.value); next.model = model;
    next.model_entities = selectedModel(model)?.entity_types ?? [];
    next.enabled_entities = next.enabled_entities.filter(type => supplementaryEntities.includes(type as typeof supplementaryEntities[number]));
    draft.value = next; modelDrafts.set(model, clone(next));
  }
}
function selectAllModelEntities() { draft.value.model_entities = [...modelEntities.value]; }
function clearModelEntities() { draft.value.model_entities = []; }
function addRule(kind: CustomRule["kind"]) {
  const base = { id: crypto.randomUUID(), entity_type: "CUSTOM", enabled: true };
  draft.value.custom_rules.push(kind === "regex"
    ? { ...base, kind, pattern: "" }
    : { ...base, kind, words: [""] });
}
function submit(preview = false) {
  if (props.busy || validation.value || (preview && !previewText.value.length)) return;
  const snapshot = ProcessingConfigSchema.parse(draft.value);
  if (preview) emit("preview", props.pair.id, snapshot, previewText.value);
  else emit("save", props.pair.id, snapshot);
}
</script>

<template>
  <OnyxCard class="detection-settings" role="region" aria-labelledby="detection-heading">
    <OnyxHeadline id="detection-heading" is="h2">Erkennung für {{ pair.name }}</OnyxHeadline>
    <form @submit.prevent="submit()">
      <OnyxSelect :model-value="draft.model" label="Lokales Sprachmodell" list-label="Installierte Sprachmodelle"
        :options="modelOptions" :disabled="busy" :hide-clear-icon="true" @update:model-value="chooseModel" />
      <p>Modelle werden ausschließlich lokal verwendet. Fehlende Modelle werden nicht heruntergeladen.</p>
      <p v-if="draft.model === biomedbert">BiomedBERT läuft lokal auf der CPU; das erste Laden kann einen Moment dauern.</p>
      <fieldset class="model-entities" :disabled="busy">
        <legend>Vom Modell erkannte Labels</legend>
        <p>Die nativen Label-Codes bleiben in den Ergebnissen erhalten. Speichern erfordert die erneute Verarbeitung vorhandener Dokumente.</p>
        <div class="actions">
          <OnyxButton data-testid="select-all-model-entities" label="Alle auswählen" type="button" mode="outline" :disabled="busy" @click="selectAllModelEntities" />
          <OnyxButton data-testid="clear-model-entities" label="Auswahl löschen" type="button" mode="outline" :disabled="busy" @click="clearModelEntities" />
        </div>
        <div class="model-entity-list">
          <OnyxCheckbox v-for="entity in modelEntities" :key="entity" :data-testid="`model-entity-${entity}`"
            :label="entityOption(entity).label" :value="entity" :model-value="draft.model_entities?.includes(entity) ?? false"
            truncation="multiline" :disabled="busy" @update:model-value="toggleEntity(entity, $event, 'model')" />
        </div>
      </fieldset>
      <fieldset class="builtin-entities" :disabled="busy">
        <legend>Zusätzliche Erkenner</legend>
        <OnyxCheckbox v-for="option in supplementaryOptions" :key="option.value" :data-testid="`entity-${option.value}`"
          :label="option.label" :value="option.value" :model-value="draft.enabled_entities.includes(option.value)"
          :disabled="busy" @update:model-value="toggleEntity(option.value, $event, 'supplementary')" />
      </fieldset>
      <OnyxCheckbox v-model="draft.include_positions" data-testid="include-positions" value="positions"
        label="Detaillierte Positionen in Markdown aufnehmen" :disabled="busy" />
      <p>Die internen Positionen für die Prüfung bleiben immer erhalten.</p>
      <h3>Eigene Regeln</h3>
      <p>Reguläre Ausdrücke verwenden Python-Syntax. Wörter werden wörtlich und unter Beachtung der Groß- und Kleinschreibung erkannt. Muster werden bei Vorschau und Speichern geprüft.</p>
      <fieldset v-for="(rule, index) in draft.custom_rules" :key="rule.id" :disabled="busy">
        <legend>{{ rule.kind === "regex" ? "Regulärer Ausdruck" : "Wortliste" }} {{ index + 1 }}</legend>
        <OnyxCheckbox v-model="rule.enabled" label="Regel aktiv" :value="rule.id" :disabled="busy" />
        <OnyxSelect v-model="rule.entity_type" label="Erkannter Typ" list-label="Entitätstypen"
          :options="entityOptions" :disabled="busy" :hide-clear-icon="true" />
        <OnyxInput v-if="rule.kind === 'regex'" v-model="rule.pattern" data-testid="rule-pattern"
          label="Muster (Python-RegEx)" required :disabled="busy" />
        <OnyxTextarea v-else data-testid="rule-words" label="Wörter (ein Eintrag pro Zeile)"
          :model-value="rule.words.join('\n')" required :disabled="busy"
          @update:model-value="rule.words = ($event ?? '').split('\n')" />
        <OnyxButton data-testid="remove-rule" label="Regel entfernen" type="button" mode="outline"
          :disabled="busy" @click="draft.custom_rules.splice(index, 1)" />
      </fieldset>
      <div class="actions">
        <OnyxButton data-testid="add-regex" label="Regulären Ausdruck hinzufügen" type="button" mode="outline" :disabled="busy" @click="addRule('regex')" />
        <OnyxButton data-testid="add-words" label="Wortliste hinzufügen" type="button" mode="outline" :disabled="busy" @click="addRule('words')" />
      </div>
      <OnyxTextarea v-model="previewText" data-testid="preview-text" label="Testtext für die Vorschau" :disabled="busy" />
      <p>Verwenden Sie erfundenen Testtext. Die Vorschau speichert keine Einstellungen.</p>
      <p v-if="validation" role="alert">{{ validation }}</p>
      <p v-if="error" role="alert">{{ errors[error.code] ?? "Die Erkennung konnte nicht geprüft oder gespeichert werden. Bitte versuchen Sie es erneut." }}</p>
      <p v-if="busy" role="status">Erkennung wird geprüft …</p>
      <p v-else-if="saved && JSON.stringify(pair.config) === JSON.stringify(draft)" role="status">Erkennung gespeichert. Bei Änderungen sind vorhandene Ergebnisse veraltet; geprüfte Dokumente benötigen vor erneuter Verarbeitung Ihre Bestätigung.</p>
      <section v-if="visiblePreview" data-testid="preview-results" aria-live="polite">
        <h3>Vorschau: {{ visiblePreview.detections.length }} Treffer</h3>
        <p>Die Vorschau unterstützt die Prüfung; sie garantiert keine vollständige Erkennung.</p>
        <ul>
          <li v-for="detection in visiblePreview.detections" :key="detection.id">
            {{ entityLabel(detection.entity_type) }} ({{ detection.entity_type }}): <q>{{ Array.from(visiblePreview.text).slice(detection.start, detection.end).join('') }}</q>
            (Position {{ detection.start }}–{{ detection.end }})
          </li>
        </ul>
      </section>
      <div class="actions">
        <OnyxButton data-testid="preview" label="Vorschau prüfen" type="button" mode="outline"
          :disabled="busy || !!validation || !previewText.length" @click="submit(true)" />
        <OnyxButton data-testid="save" label="Erkennung speichern" type="submit" :disabled="busy || !!validation" />
      </div>
    </form>
  </OnyxCard>
</template>

<style scoped>
.detection-settings, form, fieldset { display: grid; gap: var(--onyx-spacing-md); }
fieldset { margin: 0; border: var(--onyx-1px-in-rem) solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-md); padding: var(--onyx-spacing-lg); min-width: 0; }
legend { padding-inline: var(--onyx-spacing-xs); font-weight: var(--onyx-font-weight-semibold); }
.builtin-entities { grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr)); gap: var(--onyx-spacing-sm) var(--onyx-spacing-lg); }
.model-entities { grid-template-columns: 1fr; }
.model-entity-list { display: grid; grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr)); gap: var(--onyx-spacing-sm) var(--onyx-spacing-lg); }
fieldset > :deep(.onyx-button) { justify-self: start; }
.detection-settings > h2, h3 { margin: 0; }
form > h3 { margin-block-start: var(--onyx-spacing-md); }
.actions { display: flex; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
p { margin: 0; }
</style>
