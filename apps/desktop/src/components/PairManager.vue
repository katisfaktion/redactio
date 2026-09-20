<script setup lang="ts">
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import { OnyxButton, OnyxCard, OnyxHeadline, OnyxInput, OnyxSelect, OnyxVisuallyHidden } from "sit-onyx";
import { computed, reactive, ref, useId } from "vue";
import type { Settings } from "../lib/contracts";

const props = defineProps<{ settings: Settings; busy: boolean }>();
const emit = defineEmits<{
  add: [name: string, source: string, target: string, createTarget: boolean];
  rename: [pairId: string, name: string];
  select: [pairId: string];
  remove: [pairId: string];
}>();

const name = ref("");
const source = ref("");
const target = ref("");
const createTarget = ref(false);
const managementOpen = ref(false);
const managementId = useId();
const renameNames = reactive<Record<string, string>>({});
const pairOptions = computed(() => props.settings.sync_pairs.map((pair) => ({
  label: pair.name,
  value: pair.id,
})));

async function chooseSource() {
  const chosen = await open({ directory: true, multiple: false, title: "Quellordner auswählen" });
  if (chosen) source.value = chosen;
}

async function chooseTarget() {
  const chosen = await open({ directory: true, multiple: false, title: "Zielordner auswählen" });
  if (chosen) {
    target.value = chosen;
    createTarget.value = false;
  }
}

async function chooseNewTarget() {
  const chosen = await save({ title: "Neuen Zielordner festlegen" });
  if (!chosen) return;
  const accepted = await confirm("Soll dieser neue Zielordner angelegt werden?", {
    title: "Zielordner anlegen",
    kind: "warning",
  });
  if (accepted) {
    target.value = chosen;
    createTarget.value = true;
  }
}

function submit() {
  emit("add", name.value, source.value, target.value, createTarget.value);
}

function selectPair(value?: string | number | null) {
  if (typeof value === "string") emit("select", value);
}

async function remove(pairId: string) {
  if (await confirm("Das Ordnerpaar aus der App entfernen? Dateien und Ergebnisse bleiben erhalten.", {
    title: "Ordnerpaar entfernen",
    kind: "warning",
  })) emit("remove", pairId);
}
</script>

<template>
  <OnyxCard class="pair-manager" role="region" aria-labelledby="pairs-heading">
    <OnyxVisuallyHidden v-if="settings.sync_pairs.length" id="pairs-heading" is="h2">Ordnerpaare</OnyxVisuallyHidden>
    <OnyxHeadline v-else id="pairs-heading" is="h2" show-as="h3">Ordnerpaare</OnyxHeadline>
    <div v-if="settings.sync_pairs.length" class="pair-toolbar">
      <OnyxSelect
        class="pair-selector"
        data-testid="active-pair-select"
        label="Aktives Ordnerpaar"
        list-label="Gespeicherte Ordnerpaare"
        :model-value="settings.selected_sync_pair_id ?? undefined"
        :options="pairOptions"
        :hide-clear-icon="true"
        :disabled="busy"
        @update:model-value="selectPair"
      />
      <OnyxButton
        data-testid="pair-management-toggle"
        label="Ordnerpaare verwalten"
        type="button"
        color="neutral"
        :mode="managementOpen ? 'default' : 'outline'"
        :aria-expanded="managementOpen"
        :aria-controls="managementId"
        :disabled="busy"
        @click="managementOpen = !managementOpen"
      />
    </div>
    <slot name="context" />

    <div v-show="managementOpen || !settings.sync_pairs.length" :id="managementId" class="management" :class="{ 'management--separated': settings.sync_pairs.length }">

      <ul v-if="settings.sync_pairs.length" class="pairs">
        <li v-for="pair in settings.sync_pairs" :key="pair.id">
          <div>
            <strong>{{ pair.name }}</strong>
            <small>{{ pair.source_folder }} → {{ pair.target_folder }}</small>
          </div>
          <div class="pair-actions">
            <OnyxInput
              v-model="renameNames[pair.id]"
              :label="`Neuer Name für ${pair.name}`"
              :placeholder="pair.name"
              :disabled="busy"
            />
            <OnyxButton
              label="Umbenennen"
              type="button"
              mode="outline"
              :disabled="busy || !renameNames[pair.id]?.trim()"
              @click="emit('rename', pair.id, renameNames[pair.id])"
            />
            <OnyxButton
              label="Entfernen"
              type="button"
              color="danger"
              mode="outline"
              :disabled="busy"
              @click="remove(pair.id)"
            />
          </div>
        </li>
      </ul>

      <form class="add-pair" @submit.prevent="submit">
        <h3>{{ settings.sync_pairs.length ? "Weiteres Ordnerpaar" : "Noch kein Ordnerpaar eingerichtet" }}</h3>
        <p v-if="!settings.sync_pairs.length">Wählen Sie einen Quellordner und einen getrennten Zielordner aus.</p>
        <OnyxInput v-model="name" label="Name" required :disabled="busy" />
        <div class="folder-choice">
          <OnyxButton label="Quellordner auswählen" type="button" mode="outline" :disabled="busy" @click="chooseSource" />
          <span>{{ source || "Kein Quellordner gewählt" }}</span>
        </div>
        <div class="folder-choice">
          <OnyxButton label="Bestehenden Zielordner auswählen" type="button" mode="outline" :disabled="busy" @click="chooseTarget" />
          <OnyxButton label="Neuen Zielordner festlegen" type="button" mode="outline" :disabled="busy" @click="chooseNewTarget" />
          <span>{{ target || "Kein Zielordner gewählt" }}</span>
        </div>
        <p class="hint">Ein nicht leerer Zielordner wird nur übernommen, wenn er bereits zu dieser Quelle gehört.</p>
        <OnyxButton label="Ordnerpaar hinzufügen" type="submit" :disabled="busy || !name.trim() || !source || !target" />
      </form>
    </div>
  </OnyxCard>
</template>

<style scoped>
.pair-manager, .management, .add-pair { display: grid; gap: var(--onyx-spacing-lg); min-width: 0; }
.pair-toolbar { display: flex; flex-wrap: wrap; align-items: flex-end; gap: var(--onyx-spacing-md); }
.pair-selector { flex: 1 1 20rem; max-width: 28rem; min-width: 0; }
.management--separated { border-top: 1px solid var(--onyx-color-component-border-neutral); padding-block-start: var(--onyx-spacing-lg); }
.pairs { display: grid; gap: var(--onyx-spacing-md); list-style: none; margin: 0; padding: 0; }
.pairs li { display: grid; gap: var(--onyx-spacing-md); padding-block: var(--onyx-spacing-md); border-bottom: 1px solid var(--onyx-color-component-border-neutral); }
.pair-actions, .folder-choice { display: flex; flex-wrap: wrap; align-items: flex-end; gap: var(--onyx-spacing-sm); }
.pair-actions :deep(.onyx-input) { flex: 1 1 16rem; max-width: 28rem; }
.folder-choice span { flex-basis: 100%; overflow-wrap: anywhere; }
.pairs small { display: block; overflow-wrap: anywhere; color: var(--onyx-color-text-icons-neutral-medium); }
.add-pair > :deep(.onyx-input) { max-width: 32rem; }
.add-pair > :deep(.onyx-button) { justify-self: start; }
.hint { color: var(--onyx-color-text-icons-neutral-medium); font-size: var(--onyx-font-size-sm); }
h2, h3, p { margin: 0; }
</style>
