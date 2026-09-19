<script setup lang="ts">
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import { reactive, ref } from "vue";
import type { Settings } from "../lib/contracts";

defineProps<{ settings: Settings; busy: boolean }>();
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
const renameNames = reactive<Record<string, string>>({});

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

async function remove(pairId: string) {
  if (await confirm("Das Ordnerpaar aus der App entfernen? Dateien und Ergebnisse bleiben erhalten.", {
    title: "Ordnerpaar entfernen",
    kind: "warning",
  })) emit("remove", pairId);
}
</script>

<template>
  <section class="pair-manager" aria-labelledby="pairs-heading">
    <h2 id="pairs-heading">Ordnerpaare</h2>

    <label v-if="settings.sync_pairs.length">
      Aktives Ordnerpaar
      <select :value="settings.selected_sync_pair_id ?? ''" :disabled="busy" @change="emit('select', ($event.target as HTMLSelectElement).value)">
        <option v-for="pair in settings.sync_pairs" :key="pair.id" :value="pair.id">{{ pair.name }}</option>
      </select>
    </label>

    <ul v-if="settings.sync_pairs.length" class="pairs">
      <li v-for="pair in settings.sync_pairs" :key="pair.id">
        <div>
          <strong>{{ pair.name }}</strong>
          <small>{{ pair.source_folder }} → {{ pair.target_folder }}</small>
        </div>
        <div class="pair-actions">
          <input v-model="renameNames[pair.id]" :placeholder="pair.name" :aria-label="`Neuer Name für ${pair.name}`" :disabled="busy">
          <button type="button" :disabled="busy || !renameNames[pair.id]?.trim()" @click="emit('rename', pair.id, renameNames[pair.id])">Umbenennen</button>
          <button type="button" :disabled="busy" @click="remove(pair.id)">Entfernen</button>
        </div>
      </li>
    </ul>

    <form class="add-pair" @submit.prevent="submit">
      <h3>{{ settings.sync_pairs.length ? "Weiteres Ordnerpaar" : "Noch kein Ordnerpaar eingerichtet" }}</h3>
      <p v-if="!settings.sync_pairs.length">Wählen Sie einen Quellordner und einen getrennten Zielordner aus.</p>
      <label>Name <input v-model="name" required :disabled="busy"></label>
      <div class="folder-choice">
        <button type="button" :disabled="busy" @click="chooseSource">Quellordner auswählen</button>
        <span>{{ source || "Kein Quellordner gewählt" }}</span>
      </div>
      <div class="folder-choice">
        <button type="button" :disabled="busy" @click="chooseTarget">Bestehenden Zielordner auswählen</button>
        <button type="button" :disabled="busy" @click="chooseNewTarget">Neuen Zielordner festlegen</button>
        <span>{{ target || "Kein Zielordner gewählt" }}</span>
      </div>
      <p class="hint">Ein nicht leerer Zielordner wird nur übernommen, wenn er bereits zu dieser Quelle gehört.</p>
      <button type="submit" :disabled="busy || !name.trim() || !source || !target">Ordnerpaar hinzufügen</button>
    </form>
  </section>
</template>

<style scoped>
.pair-manager, .add-pair { display: grid; gap: var(--onyx-spacing-lg); }
.pairs { display: grid; gap: var(--onyx-spacing-md); list-style: none; margin: 0; padding: 0; }
.pairs li, .pair-actions, .folder-choice { align-items: center; display: flex; flex-wrap: wrap; gap: var(--onyx-spacing-sm); justify-content: space-between; }
.pairs small { display: block; }
.hint { color: var(--onyx-color-text-icons-neutral-medium); font-size: var(--onyx-font-size-sm); }
label { display: grid; gap: var(--onyx-spacing-xs); }
</style>
