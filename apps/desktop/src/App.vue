<script setup lang="ts">
import { OnyxAppLayout, OnyxButton, OnyxModal, OnyxPageLayout, OnyxSelect } from "sit-onyx";
import { computed, onMounted, onUnmounted, ref, shallowRef, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm } from "@tauri-apps/plugin-dialog";
import MappingRecovery from "./components/MappingRecovery.vue";
import DocumentList from "./components/DocumentList.vue";
import PairManager from "./components/PairManager.vue";
import RunPanel from "./components/RunPanel.vue";
import DetectionSettings from "./components/DetectionSettings.vue";
import ReviewView from "./components/ReviewView.vue";
import { useReview } from "./composables/useReview";
import { useDetection } from "./composables/useDetection";
import { usePairs } from "./composables/usePairs";
import { useRun } from "./composables/useRun";
import { runApi, safeError } from "./lib/ipc";
import type { SafeError, ScanReport, Settings } from "./lib/contracts";

const props = withDefaults(defineProps<{
  initialSettings: Settings | null;
  initialError?: SafeError | null;
}>(), { initialError: null });

const startupError = ref(props.initialError);
function recovered(settings: Settings) { pairs.settings.value = settings; startupError.value = null; }

const empty: Settings = { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null };
const pairs = usePairs(props.initialSettings ?? empty);
const detection = useDetection(pairs.selectedPair, (settings) => { pairs.settings.value = settings; });
const view = ref<"documents" | "settings" | "review">("documents");
const review = useReview();
const reviewDocuments = ref<{ value: string; label: string }[]>([]);
const selectedId = computed(() => pairs.selectedPair.value?.id ?? null);
const run = useRun(selectedId);
const scanned = ref(false), confirming = ref(false);
const busy = computed(() => pairs.busy.value || run.busy.value || detection.busy.value || review.busy.value || confirming.value);
const leaveAction = shallowRef<(() => void | Promise<void>) | null>(null);
const closeError = ref("");
function guard(action: () => void | Promise<void>) {
  if (busy.value || leaveAction.value) return;
  if (review.dirty.value) leaveAction.value = action;
  else void action();
}
async function leave(choice: "save" | "discard" | "stay") {
  if (busy.value) return;
  if (choice === "stay") { leaveAction.value = null; return; }
  if (choice === "save" && !await review.save()) return;
  const action = leaveAction.value; leaveAction.value = null;
  review.clear(); await action?.();
}
function changeView(next: "documents" | "settings") {
  if (next === view.value) return;
  guard(() => { review.clear(); view.value = next; if (next === "settings") void loadAuditLocation(); });
}
function changePair(action: () => Promise<void>) {
  guard(async () => { review.clear(); view.value = "documents"; await action(); });
}
function openReview(docId: string) {
  const pairId = selectedId.value;
  if (!pairId || (view.value === "review" && review.key.value?.doc_id === docId)) return;
  guard(async () => { view.value = "review"; await review.open({ sync_pair_id: pairId, doc_id: docId }); });
}
function onScanned(report: ScanReport) {
  scanned.value = true;
  reviewDocuments.value = report.files.flatMap(file => file.doc_id && file.state === "current" ? [{ value: file.doc_id, label: `${file.doc_id} · ${file.relative_path}` }] : []);
}
let disposed = false, unlistenClose: (() => void) | undefined;
onMounted(async () => {
  try {
    const unlisten = await getCurrentWindow().onCloseRequested(event => {
      if (busy.value) { event.preventDefault(); closeError.value = "Bitte warten Sie, bis der laufende Vorgang abgeschlossen ist."; return; }
      if (!review.dirty.value && !leaveAction.value) return;
      event.preventDefault();
      guard(async () => {
        try { await getCurrentWindow().destroy(); }
        catch { closeError.value = "Das Fenster konnte nicht geschlossen werden."; }
      });
    });
    if (disposed) unlisten(); else unlistenClose = unlisten;
  } catch { if (!disposed) closeError.value = "Der Schutz beim Schließen ist nicht verfügbar. Speichern Sie Änderungen vor dem Schließen."; }
});
onUnmounted(() => { disposed = true; unlistenClose?.(); leaveAction.value = null; });
const emptyCounts = { discovered: 0, processed: 0, skipped: 0, failed: 0, unprocessed: 0, warned: 0 };
watch(selectedId, () => { scanned.value = false; reviewDocuments.value = []; review.clear(); });
watch(() => pairs.selectedPair.value?.processing_revision, () => { scanned.value = false; });
const auditPath = ref("");
const auditError = ref(false);
async function loadAuditLocation() {
  try { auditPath.value = await runApi.auditLocation(); auditError.value = false; }
  catch { auditError.value = true; }
}
async function openAuditFolder() {
  try { await runApi.openAuditFolder(); auditError.value = false; }
  catch { auditError.value = true; }
}
async function reprocess(files: { relative_path: string; doc_id: string }[]) {
  if (busy.value || !files.length) return;
  const pairId = selectedId.value;
  confirming.value = true;
  try {
    const accepted = await confirm(`Diese Dokumente erneut verarbeiten? Gespeicherte Prüfungen und manuelle Korrekturen werden verworfen:\n\n${files.map((file) => file.relative_path).join("\n")}`, { title: "Erneut verarbeiten", kind: "warning" });
    if (accepted && selectedId.value === pairId) await run.start(files.map((file) => file.relative_path), files.map((file) => file.doc_id));
  } catch (caught) { run.error.value = safeError(caught); }
  finally { confirming.value = false; }
}
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
            :busy="busy"
            @add="(name, source, target, createTarget) => changePair(() => pairs.addPair(name, source, target, createTarget))"
            @rename="pairs.renamePair"
            @select="id => id !== selectedId && changePair(() => pairs.selectPair(id))"
            @remove="id => changePair(() => pairs.removePair(id))"
          >
            <template #context><small v-if="view === 'review'">Prüfung: {{ review.key.value?.doc_id }}</small></template>
          </PairManager>
          <nav aria-label="Ansichten" class="view-navigation">
            <OnyxButton data-testid="documents-nav" label="Dokumente" type="button" :mode="view === 'documents' ? 'default' : 'outline'" :aria-current="view === 'documents' ? 'page' : undefined" :disabled="busy" @click="changeView('documents')" />
            <OnyxButton data-testid="settings-nav" label="Einstellungen" type="button" :mode="view === 'settings' ? 'default' : 'outline'" :aria-current="view === 'settings' ? 'page' : undefined" :disabled="busy" @click="changeView('settings')" />
          </nav>
          <p v-if="closeError" role="alert">{{ closeError }}</p>
          <template v-if="view === 'review' && pairs.selectedPair.value">
            <OnyxSelect data-testid="review-document-select" label="Dokument prüfen" list-label="Verarbeitete Dokumente" :options="reviewDocuments" :model-value="review.key.value?.doc_id" :hide-clear-icon="true" :disabled="busy" @update:model-value="id => typeof id === 'string' && openReview(id)" />
            <ReviewView :key="`${review.key.value?.sync_pair_id}:${review.key.value?.doc_id}`" :review="review" :pair-name="pairs.selectedPair.value.name" @back="changeView('documents')" />
          </template>
          <p v-if="view === 'documents' && detection.error.value" role="alert">Die Erkennung ist derzeit nicht verfügbar. Prüfen Sie die Modelle und Regeln unter Einstellungen.</p>
          <section v-if="pairs.selectedPair.value" v-show="view === 'documents'" data-testid="document-view" aria-labelledby="documents-heading">
            <h2 id="documents-heading">Dokumente: {{ pairs.selectedPair.value.name }}</h2>
            <p>Der Arbeitsordner enthält auch ungeprüfte Ergebnisse. Prüfen Sie Dokumente vor der Weitergabe.</p>
            <DocumentList :key="`${pairs.selectedPair.value.id}:${pairs.selectedPair.value.processing_revision}`" :pair-id="pairs.selectedPair.value.id" :disabled="busy" @scanned="onScanned" @reprocess="reprocess" @review="openReview" />
            <RunPanel
              :pair-name="pairs.selectedPair.value.name"
              :counts="run.progress.value ?? emptyCounts"
              :stage="run.progress.value?.stage ?? (run.busy.value ? 'initializing' : null)"
              :outcome="run.summary.value?.outcome"
              :cancelling="run.cancelling.value"
              :disabled="busy || !scanned"
              :error="run.error.value"
              :errors="run.summary.value?.errors"
              :audit-warning="run.summary.value?.audit_warning"
              @start="run.start()" @cancel="run.cancel()"
            />
            <OnyxButton v-if="run.summary.value?.errors.length" label="Fehlgeschlagene Dokumente erneut versuchen" type="button" :disabled="busy" @click="run.start(run.summary.value.errors.map((failure) => failure.relative_path))" />
          </section>
          <section v-if="view === 'settings'" class="settings" aria-label="Einstellungen">
            <DetectionSettings v-if="pairs.selectedPair.value" :pair="pairs.selectedPair.value" :models="detection.models.value" :busy="busy"
              :preview="detection.result.value" :error="detection.error.value" :saved="detection.saved.value"
              @save="detection.save" @preview="detection.preview" />
            <h2>Protokoll</h2>
            <p>{{ auditPath }}</p>
            <OnyxButton label="Protokollordner öffnen" type="button" @click="openAuditFolder" />
            <p v-if="auditError" role="alert">Der Protokollordner ist derzeit nicht verfügbar.</p>
          </section>
        </template>
        <OnyxModal label="Ungespeicherte Prüfung" :open="!!leaveAction" :alert="true" @update:open="open => !open && leave('stay')">
          <div class="leave-dialog">
            <h2>Ungespeicherte Prüfung</h2>
            <p>Änderungen vor dem Verlassen speichern oder verwerfen?</p>
            <p v-if="review.error.value" role="alert">Speichern fehlgeschlagen. Ihre Änderungen bleiben erhalten.</p>
            <div class="view-navigation">
              <OnyxButton data-testid="leave-save" label="Speichern und fortfahren" type="button" :disabled="busy" @click="leave('save')" />
              <OnyxButton data-testid="leave-discard" label="Änderungen verwerfen" type="button" mode="outline" :disabled="busy" @click="leave('discard')" />
              <OnyxButton data-testid="leave-stay" label="Hier bleiben" type="button" mode="outline" :disabled="busy" autofocus @click="leave('stay')" />
            </div>
          </div>
        </OnyxModal>
      </main>
    </OnyxPageLayout>
  </OnyxAppLayout>
</template>

<style scoped>
.shell { display: grid; gap: var(--onyx-spacing-lg); }
.shell { min-height: 100%; padding-block: var(--onyx-spacing-md); }
.eyebrow { font-weight: var(--onyx-font-weight-semibold); margin: 0 0 var(--onyx-spacing-xs); }
h1, h2, h3, p { margin-block: 0; }
:deep(.pair-manager), .error, [data-testid="document-view"] { background: var(--onyx-color-base-background-blank); border: var(--onyx-1px-in-rem) solid var(--onyx-color-component-border-neutral); border-radius: var(--onyx-radius-md); padding: var(--onyx-spacing-xl); }
.error { color: var(--onyx-color-text-icons-danger-intense); }
.view-navigation { display: flex; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
.leave-dialog { display: grid; gap: var(--onyx-spacing-md); padding: var(--onyx-spacing-lg); max-width: 36rem; }
.settings { display: grid; gap: var(--onyx-spacing-lg); }
:deep(.pair-manager) { position: sticky; top: 0; z-index: 1; align-self: start; padding: var(--onyx-spacing-md); }
</style>
