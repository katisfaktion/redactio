<script setup lang="ts">
import { OnyxButton, OnyxCheckbox, OnyxInput, OnyxSelect, OnyxTextarea } from "sit-onyx";
import { computed, ref, watch } from "vue";
import { EntityTypeSchema, ProcessingConfigSchema, type CustomRule, type EntityType, type ProcessingConfig, type SyncPair } from "../lib/contracts";

const props = defineProps<{
  pair: SyncPair;
  models: { name: string; version: string; compatible: boolean }[];
  busy: boolean;
}>();
const emit = defineEmits<{
  save: [pairId: string, config: ProcessingConfig];
  preview: [pairId: string, config: ProcessingConfig, text: string];
}>();
const draft = ref(ProcessingConfigSchema.parse(props.pair.config));
const previewText = ref("");
watch(() => [props.pair.id, props.pair.config], () => {
  draft.value = ProcessingConfigSchema.parse(props.pair.config);
  previewText.value = "";
});

const labels: Record<EntityType, string> = {
  PERSON: "Personen", LOCATION: "Orte", EMAIL_ADDRESS: "E-Mail-Adressen",
  PHONE_NUMBER: "Telefonnummern", IBAN_CODE: "IBAN", IP_ADDRESS: "IP-Adressen",
  URL: "Webadressen", DATE_TIME: "Datum und Uhrzeit", CUSTOM: "Eigener Typ",
};
const entityOptions = EntityTypeSchema.options.map(value => ({ value, label: labels[value] }));
const builtins = entityOptions.filter(option => option.value !== "CUSTOM");
const modelOptions = computed(() => props.models.map(model => ({
  value: model.name, label: `${model.name} (${model.version})${model.compatible ? "" : " – nicht verfügbar"}`,
  disabled: !model.compatible,
})));
const validation = computed(() => {
  if (!props.models.some(model => model.name === draft.value.model && model.compatible)) {
    return "Das gewählte Modell ist lokal nicht verfügbar oder nicht kompatibel. Wählen Sie ein verfügbares Modell aus.";
  }
  if (draft.value.custom_rules.some(rule => rule.kind === "regex" ? !rule.pattern.length : !rule.words.length || rule.words.some(word => !word.length))) {
    return "Regeln benötigen ein Muster oder eine Wortliste ohne leere Einträge.";
  }
  return "";
});

function toggleEntity(entity: EntityType, enabled: boolean | null | undefined) {
  draft.value.enabled_entities = enabled
    ? [...draft.value.enabled_entities, entity]
    : draft.value.enabled_entities.filter(value => value !== entity);
}
function addRule(kind: CustomRule["kind"]) {
  const base = { id: crypto.randomUUID(), entity_type: "CUSTOM" as const, enabled: true };
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
  <section class="detection-settings" aria-labelledby="detection-heading">
    <h2 id="detection-heading">Erkennung für {{ pair.name }}</h2>
    <form @submit.prevent="submit()">
      <OnyxSelect v-model="draft.model" label="Lokales Sprachmodell" list-label="Installierte Sprachmodelle"
        :options="modelOptions" :disabled="busy" :hide-clear-icon="true" />
      <p>Modelle werden ausschließlich lokal verwendet. Fehlende Modelle werden nicht heruntergeladen.</p>
      <fieldset :disabled="busy">
        <legend>Automatische Erkennung</legend>
        <OnyxCheckbox v-for="option in builtins" :key="option.value" :data-testid="`entity-${option.value}`"
          :label="option.label" :value="option.value" :model-value="draft.enabled_entities.includes(option.value)"
          :disabled="busy" @update:model-value="toggleEntity(option.value, $event)" />
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
      <div class="actions">
        <OnyxButton data-testid="preview" label="Vorschau prüfen" type="button" mode="outline"
          :disabled="busy || !!validation || !previewText.length" @click="submit(true)" />
        <OnyxButton data-testid="save" label="Erkennung speichern" type="submit" :disabled="busy || !!validation" />
      </div>
    </form>
  </section>
</template>

<style scoped>
.detection-settings, form, fieldset { display: grid; gap: var(--onyx-spacing-md); }
fieldset { margin: 0; border: var(--onyx-1px-in-rem) solid var(--onyx-color-component-border-neutral); padding: var(--onyx-spacing-md); min-width: 0; }
.actions { display: flex; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
p { margin: 0; }
</style>
