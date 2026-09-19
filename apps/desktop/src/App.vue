<script setup lang="ts">
import { OnyxAppLayout, OnyxPageLayout } from "sit-onyx";
import { ref } from "vue";
import MappingRecovery from "./components/MappingRecovery.vue";
import DocumentList from "./components/DocumentList.vue";
import PairManager from "./components/PairManager.vue";
import { usePairs } from "./composables/usePairs";
import type { SafeError, Settings } from "./lib/contracts";

const props = withDefaults(defineProps<{
  initialSettings: Settings | null;
  initialError?: SafeError | null;
}>(), { initialError: null });

const startupError = ref(props.initialError);
function recovered(settings: Settings) { pairs.settings.value = settings; startupError.value = null; }

const empty: Settings = { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null };
const pairs = usePairs(props.initialSettings ?? empty);
const errorText: Record<string, string> = {
  invalid_settings: "Einstellungen konnten nicht geladen werden. Die vorhandene Datei wurde nicht verändert.",
  mapping_recovery_required: "Eine unterbrochene Wiederherstellung muss fortgesetzt werden.",
  invalid_mapping: "Die Zuordnungsdatei ist ungültig und wurde nicht verändert.",
  mapping_missing: "Die Zuordnungsdatei fehlt. Das Ordnerpaar muss repariert werden, bevor es verwendet werden kann.",
  mapping_pair_mismatch: "Die Identität des Ordnerpaars stimmt nicht mit der Zuordnungsdatei überein. Das Ordnerpaar muss repariert werden.",
  mapping_target_mismatch: "Der Zielordner stimmt nicht mit der Zuordnungsdatei überein. Das Ordnerpaar muss repariert werden.",
  invalid_pair_name: "Bitte geben Sie einen Namen ein.",
  duplicate_pair_name: "Dieser Name wird bereits verwendet.",
  target_not_empty: "Der Zielordner ist nicht leer und gehört noch nicht zu dieser Quelle.",
  confirmation_required: "Das Anlegen des Zielordners muss bestätigt werden.",
};
</script>

<template>
  <OnyxAppLayout>
    <OnyxPageLayout>
      <main class="shell">
        <header>
          <p class="eyebrow">Redactio</p>
          <h1>Dokumente sicher schwärzen</h1>
        </header>

        <section v-if="startupError" class="error" role="alert">
          <h2>Einstellungen konnten nicht geladen werden</h2>
          <p>{{ errorText[startupError.code] ?? "Die lokalen Einstellungen sind derzeit nicht verfügbar." }}</p>
          <MappingRecovery v-if="startupError.code !== 'invalid_settings'" @recovered="recovered" />
        </section>

        <template v-else>
          <p v-if="pairs.error.value" class="error" role="alert">
            {{ errorText[pairs.error.value.code] ?? "Die Änderung konnte nicht gespeichert werden." }}
          </p>
          <PairManager
            :settings="pairs.settings.value"
            :busy="pairs.busy.value"
            @add="pairs.addPair"
            @rename="pairs.renamePair"
            @select="pairs.selectPair"
            @remove="pairs.removePair"
          />
          <section v-if="pairs.selectedPair.value" data-testid="document-view" aria-labelledby="documents-heading">
            <h2 id="documents-heading">Dokumente: {{ pairs.selectedPair.value.name }}</h2>
            <DocumentList :pair-id="pairs.selectedPair.value.id" />
          </section>
        </template>
      </main>
    </OnyxPageLayout>
  </OnyxAppLayout>
</template>

<style scoped>
.shell { display: grid; gap: var(--onyx-spacing-lg); }
.shell { min-height: 100%; padding-block: var(--onyx-spacing-2xl); }
.eyebrow { font-weight: var(--onyx-font-weight-semibold); margin: 0 0 var(--onyx-spacing-xs); }
h1, h2, h3, p { margin-block: 0; }
:deep(.pair-manager), .error, [data-testid="document-view"] { background: var(--onyx-color-base-background-blank); border: var(--onyx-1px-in-rem) solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-md); padding: var(--onyx-spacing-xl); }
.error { color: var(--onyx-color-text-icons-danger-intense); }
</style>
