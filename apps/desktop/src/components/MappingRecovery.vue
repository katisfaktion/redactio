<script setup lang="ts">
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { OnyxButton } from "sit-onyx";
import { onMounted, ref } from "vue";
import type { RecoveryPair, Settings } from "../lib/contracts";
import { pairApi } from "../lib/ipc";

const emit = defineEmits<{ recovered: [settings: Settings] }>();
const candidates = ref<RecoveryPair[]>([]);
const busy = ref(false);
const failed = ref(false);

async function reload() {
  try { candidates.value = await pairApi.recoveryPairs(); }
  catch { failed.value = true; }
}
onMounted(reload);

async function recover(pair: RecoveryPair) {
  busy.value = true;
  failed.value = false;
  try {
    const destination = pair.pending_target ?? await open({ directory: true, multiple: false, title: "Leeren Zielordner für neue Sammlung auswählen" });
    if (!destination) return;
    const accepted = await confirm(
      pair.pending_target
        ? `Die unterbrochene Wiederherstellung nach ${destination} fortsetzen?`
        : `Eine neue Sammlung im leeren Zielordner ${destination} starten? Die bisherige Sammlung wird stillgelegt. Originale und alter Zielordner bleiben erhalten. Private Zuordnungs- und Prüfdaten werden als Sicherung im Quellordner aufbewahrt.`,
      { title: "Sammlung wiederherstellen", kind: "warning" },
    );
    if (!accepted) return;
    await pairApi.freshStart(pair.id, destination, true);
    emit("recovered", await pairApi.listPairs());
  } catch {
    failed.value = true;
    // An atomic replacement may have succeeded before its durability check failed.
    try { emit("recovered", await pairApi.listPairs()); }
    catch { await reload(); }
  } finally { busy.value = false; }
}
</script>

<template>
  <section aria-label="Zuordnung wiederherstellen">
    <p>Für eine neue Sammlung wählen Sie einen anderen, leeren Zielordner. Vorhandene Ergebnisse und Originale bleiben erhalten.</p>
    <p v-if="failed" role="alert">Die Wiederherstellung ist noch nicht abgeschlossen. Sicherungen bleiben erhalten. Prüfen Sie die Ordner und versuchen Sie es erneut.</p>
    <ul>
      <li v-for="pair in candidates" :key="pair.id">
        <strong>{{ pair.name }}</strong>
        <p>{{ pair.source_folder }} → {{ pair.target_folder }}</p>
        <OnyxButton :label="pair.pending_target ? 'Wiederherstellung fortsetzen' : 'Neue Sammlung in leerem Zielordner starten'" type="button" :disabled="busy" @click="recover(pair)" />
      </li>
    </ul>
  </section>
</template>
